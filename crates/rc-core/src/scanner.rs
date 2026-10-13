//! 源目录清单扫描 + 目标目录增量进度轮询。
//!
//! 对应 Python 版 `core/scanner.py`。
//!
//! 为什么需要这一层：robocopy 的逐文件进度条只在直接写控制台时输出，stdout
//! 被重定向后就没有了，所以「实时进度」必须自己算：
//!
//! 1. 复制前扫描源目录，建立 {相对路径: (大小, 修改时间)} 清单与总字节数；
//! 2. 运行期按固定间隔扫描目标目录，把「大小已与清单一致」的文件计入已完成；
//! 3. 只有大小完全匹配才计入，避免把写了一半的文件算成已完成。
//!
//! 为了不让轮询本身成为瓶颈，做了四件事（hot / dirty / stat_budget / prune）：
//! - 只扫描源清单涉及的那棵子树，绝不遍历整个目标盘；
//! - 目录 mtime 未变化且该目录没有在写文件时，直接复用上次结果；
//! - 优先扫描 robocopy 输出中刚出现过的目录（hint），其余目录按固定预算轮转；
//! - 已全部完成的目录移出轮询集合。

use crate::winfs::{abspath, long_path, norm_for_compare};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// exFAT / FAT32 的时间戳粒度是 2 秒，比较修改时间必须给容差
pub const MTIME_TOLERANCE_NS: i64 = 2_000_000_000;

/// 单次轮询最多 stat 多少个目录（控制轮询开销上限）
pub const DEFAULT_STAT_BUDGET: usize = 3000;

/// 跳过项最多记录多少条，避免异常目录海量刷屏
pub const MAX_SKIPPED_RECORDS: usize = 200;

// ---------------------------------------------------------------------- 源扫描

/// 源清单中的一个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    /// 相对源根目录的路径（使用系统分隔符）
    pub rel: String,
    pub size: u64,
    pub mtime_ns: i64,
}

/// 源目录扫描结果。
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub root: String,
    pub files: HashMap<String, SourceEntry>,
    pub dirs: Vec<String>,
    pub total_bytes: u64,
    pub file_count: usize,
    pub skipped: Vec<String>,
    pub skipped_count: usize,
    pub cancelled: bool,
}

/// 文件系统元数据的修改时间（Unix 纪元纳秒）。
pub fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn is_under(path: &str, parent: &str) -> bool {
    let a = norm_for_compare(path);
    let b = norm_for_compare(parent);
    a == b || a.starts_with(&format!("{}\\", b))
}

fn _record_skip(result: &mut ScanResult, message: String) {
    result.skipped_count += 1;
    if result.skipped.len() < MAX_SKIPPED_RECORDS {
        result.skipped.push(message);
    }
}

/// 递归扫描源目录，返回文件清单与总字节数。
///
/// `progress_cb(已发现文件数, 已发现总字节)` 会被节流调用，用于界面提示。
/// `excludes` 中的目录（绝对路径）会被整棵跳过（例如本程序自己的日志目录）。
pub fn scan_source(
    root: &str,
    cancel: Option<&AtomicBool>,
    excludes: &[String],
    mut progress_cb: Option<&mut dyn FnMut(usize, u64)>,
    progress_interval: f64,
) -> ScanResult {
    let root_abs = abspath(root);
    let mut result = ScanResult {
        root: root_abs.clone(),
        ..Default::default()
    };
    let exclude_norms: Vec<String> = excludes
        .iter()
        .map(|e| norm_for_compare(e))
        .filter(|e| !e.is_empty())
        .collect();

    let mut stack: Vec<String> = vec![String::new()];
    let mut last_report = Instant::now();
    let progress_interval = if progress_interval <= 0.0 {
        0.3
    } else {
        progress_interval
    };

    'outer: while let Some(rel_dir) = stack.pop() {
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                result.cancelled = true;
                break;
            }
        }
        result.dirs.push(rel_dir.clone());
        let abs_dir = if rel_dir.is_empty() {
            root_abs.clone()
        } else {
            format!("{}\\{}", root_abs, rel_dir)
        };

        if !exclude_norms.is_empty() {
            let norm = norm_for_compare(&abs_dir);
            let hit = exclude_norms
                .iter()
                .any(|e| norm == *e || norm.starts_with(&format!("{}\\", e)));
            if hit {
                continue;
            }
        }

        match std::fs::read_dir(long_path(&abs_dir)) {
            Ok(entries) => {
                for entry in entries {
                    if let Some(c) = cancel {
                        if c.load(Ordering::Relaxed) {
                            result.cancelled = true;
                            break;
                        }
                    }
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let rel = if rel_dir.is_empty() {
                        name.clone()
                    } else {
                        format!("{}\\{}", rel_dir, name)
                    };
                    match entry.file_type() {
                        Ok(ft) => {
                            // follow_symlinks=False 语义：链接本身不算文件也不算目录
                            if ft.is_symlink() {
                                continue;
                            }
                            if ft.is_dir() {
                                stack.push(rel);
                            } else if ft.is_file() {
                                match entry.metadata() {
                                    Ok(stat) => {
                                        let size = stat.len();
                                        result.files.insert(
                                            rel.clone(),
                                            SourceEntry {
                                                rel,
                                                size,
                                                mtime_ns: mtime_ns(&stat),
                                            },
                                        );
                                        result.total_bytes += size;
                                    }
                                    Err(exc) => _record_skip(
                                        &mut result,
                                        format!("{}  ({})", rel, exc),
                                    ),
                                }
                            }
                        }
                        Err(exc) => {
                            _record_skip(&mut result, format!("{}  ({})", rel, exc));
                        }
                    }
                }
            }
            Err(exc) => {
                _record_skip(&mut result, format!("{}  ({})", abs_dir, exc));
            }
        }

        if result.cancelled {
            break 'outer;
        }
        if let Some(cb) = progress_cb.as_mut() {
            if last_report.elapsed().as_secs_f64() >= progress_interval {
                last_report = Instant::now();
                cb(result.files.len(), result.total_bytes);
            }
        }
    }

    result.file_count = result.files.len();
    if let Some(cb) = progress_cb.as_mut() {
        cb(result.file_count, result.total_bytes);
    }
    result
}

// ---------------------------------------------------------------------- 目标轮询

fn dirname_of(rel: &str) -> String {
    match rel.rfind(['\\', '/']) {
        Some(pos) => rel[..pos].to_string(),
        None => String::new(),
    }
}

fn basename_of(rel: &str) -> String {
    match rel.rfind(['\\', '/']) {
        Some(pos) => rel[pos + 1..].to_string(),
        None => rel.to_string(),
    }
}

/// 把绝对 path 换算为相对 base 的路径（双方均做 normcase 归一），失败返回 None。
fn relpath_to(path: &str, base: &str) -> Option<String> {
    let p = norm_for_compare(path);
    let b = norm_for_compare(base);
    if p == b {
        return Some(".".to_string());
    }
    let bsep = if b.ends_with('\\') {
        b
    } else {
        format!("{}\\", b)
    };
    p.strip_prefix(&bsep).map(|s| s.to_string())
}

struct Inner {
    dest_root: String,
    source_root: String,
    tol_ns: i64,
    stat_budget: usize,
    /// 按「目录 -> {文件名: 源条目}」重组清单
    by_dir: HashMap<String, HashMap<String, SourceEntry>>,
    /// 只扫描源清单涉及的子树
    active: Vec<String>,
    pos: usize,
    mtime: HashMap<String, i64>,
    hot: HashSet<String>,
    dirty: HashSet<String>,
    done_sizes: HashMap<String, u64>,
    done_counts: HashMap<String, u64>,
    uptodate_sizes: HashMap<String, u64>,
    uptodate_counts: HashMap<String, u64>,
    done_bytes: u64,
    done_files: u64,
    uptodate_bytes: u64,
    uptodate_files: u64,
    poll_count: u64,
    scanned_dirs: u64,
}

/// 跟踪目标目录的复制进度。
///
/// - `done_bytes`     已复制字节（目标文件大小与源清单完全一致的累计大小）
/// - `uptodate_bytes` 复制开始前就已经是最新的字节（robocopy 会跳过的部分）
///
/// 线程安全：`hint` 可被输出读取线程调用，`poll` 由轮询线程调用。
pub struct TargetProgressTracker {
    inner: Mutex<Inner>,
}

impl TargetProgressTracker {
    pub fn new(scan: &ScanResult, dest_root: &str) -> Self {
        Self::with_tolerance(scan, dest_root, MTIME_TOLERANCE_NS, DEFAULT_STAT_BUDGET)
    }

    pub fn with_tolerance(
        scan: &ScanResult,
        dest_root: &str,
        mtime_tolerance_ns: i64,
        stat_budget: usize,
    ) -> Self {
        let mut by_dir: HashMap<String, HashMap<String, SourceEntry>> = HashMap::new();
        for entry in scan.files.values() {
            by_dir
                .entry(dirname_of(&entry.rel))
                .or_default()
                .insert(basename_of(&entry.rel), entry.clone());
        }
        let mut active: Vec<String> = by_dir.keys().cloned().collect();
        active.sort();
        Self {
            inner: Mutex::new(Inner {
                dest_root: abspath(dest_root),
                source_root: scan.root.clone(),
                tol_ns: mtime_tolerance_ns,
                stat_budget: stat_budget.max(50),
                by_dir,
                active,
                pos: 0,
                mtime: HashMap::new(),
                hot: HashSet::new(),
                dirty: HashSet::new(),
                done_sizes: HashMap::new(),
                done_counts: HashMap::new(),
                uptodate_sizes: HashMap::new(),
                uptodate_counts: HashMap::new(),
                done_bytes: 0,
                done_files: 0,
                uptodate_bytes: 0,
                uptodate_files: 0,
                poll_count: 0,
                scanned_dirs: 0,
            }),
        }
    }

    // -------------------------------------------------------------- 对外接口

    pub fn active_dirs(&self) -> usize {
        self.inner.lock().unwrap().active.len()
    }

    pub fn total_dirs(&self) -> usize {
        self.inner.lock().unwrap().by_dir.len()
    }

    pub fn done_bytes(&self) -> u64 {
        self.inner.lock().unwrap().done_bytes
    }

    pub fn done_files(&self) -> u64 {
        self.inner.lock().unwrap().done_files
    }

    pub fn uptodate_bytes(&self) -> u64 {
        self.inner.lock().unwrap().uptodate_bytes
    }

    pub fn uptodate_files(&self) -> u64 {
        self.inner.lock().unwrap().uptodate_files
    }

    /// 复制开始前全量扫描一次，算出目标端已经是最新的字节数。
    ///
    /// 同时把「已经完整」的目录从轮询集合中剔除，后续轮询只关注真正要写的目录。
    pub fn build_baseline(
        &self,
        mut progress_cb: Option<&mut dyn FnMut(usize, usize)>,
    ) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let dirs: Vec<String> = inner.by_dir.keys().cloned().collect();
        let total = dirs.len();
        for (index, rel_dir) in dirs.iter().enumerate() {
            inner.scan_dir(rel_dir, false);
            if let Some(cb) = progress_cb.as_mut() {
                if (index + 1) % 200 == 0 {
                    cb(index + 1, total);
                }
            }
        }
        inner.prune();
        inner.recompute();
        if let Some(cb) = progress_cb {
            cb(total, total);
        }
        inner.done_bytes
    }

    /// 把 robocopy 输出中出现的路径登记为「热点目录」，下轮优先扫描。
    ///
    /// 路径可能是目标绝对路径、源绝对路径或相对路径，三种都尝试识别。
    pub fn hint(&self, path: &str) {
        if path.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if let Some(rel) = inner.to_rel(path) {
            let rel_dir = dirname_of(&rel);
            if inner.by_dir.contains_key(&rel_dir) {
                inner.hot.insert(rel_dir);
            }
        }
    }

    /// 执行一次增量轮询，返回当前已复制字节数。
    pub fn poll(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        inner.poll_count += 1;

        // 1) 热点目录优先：robocopy 刚动过的目录，一定值得看一眼
        let hot: Vec<String> = inner
            .hot
            .iter()
            .filter(|d| inner.by_dir.contains_key(*d))
            .cloned()
            .collect();
        inner.hot.clear();
        for rel_dir in hot {
            inner.scan_dir(&rel_dir, true);
        }

        // 2) 其余目录按预算轮转，避免一次轮询把整个目标盘翻一遍
        inner.prune();
        let count = inner.active.len();
        if count > 0 {
            let budget = inner.stat_budget.min(count);
            let mut to_scan: Vec<String> = Vec::new();
            for offset in 0..budget {
                let rel_dir = inner.active[(inner.pos + offset) % count].clone();
                let Some(mtime) = inner.dir_mtime(&rel_dir) else {
                    continue; // 目录还没被创建
                };
                if inner.mtime.get(&rel_dir).copied() != Some(mtime)
                    || inner.dirty.contains(&rel_dir)
                {
                    to_scan.push(rel_dir.clone());
                }
                inner.mtime.insert(rel_dir, mtime);
            }
            inner.pos = (inner.pos + budget) % count;
            for rel_dir in to_scan {
                inner.scan_dir(&rel_dir, true);
            }
        }

        inner.recompute();
        inner.done_bytes
    }

    /// 任务结束前的兜底终扫，保证最终数字准确。
    pub fn final_poll(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let dirs: Vec<String> = inner.by_dir.keys().cloned().collect();
        for rel_dir in dirs {
            let expected = inner.by_dir[&rel_dir].len() as u64;
            if inner.done_counts.get(&rel_dir).copied() != Some(expected) {
                inner.scan_dir(&rel_dir, false);
            }
        }
        inner.prune();
        inner.recompute();
        inner.done_bytes
    }

    /// 轮询开销自述，用于写入日志。
    pub fn diagnostics(&self) -> String {
        let inner = self.inner.lock().unwrap();
        format!(
            "进度轮询 {} 次，扫描目录 {} 次，清单目录 {} 个（剩余待轮询 {} 个）",
            inner.poll_count,
            inner.scanned_dirs,
            inner.by_dir.len(),
            inner.active.len()
        )
    }

    // -------------------------------------------------------------- 内部实现
}

impl Inner {
    fn to_rel(&self, path: &str) -> Option<String> {
        let text = path.trim().trim_matches('"');
        if text.is_empty() {
            return None;
        }
        for base in [&self.dest_root, &self.source_root] {
            if is_under(text, base) {
                let rel = relpath_to(text, base)?;
                return Some(if rel == "." { String::new() } else { rel });
            }
        }
        // 相对路径：直接用
        let rel = text.replace('/', "\\");
        let rel = rel.trim_start_matches('\\').to_string();
        if rel.starts_with("..") {
            return None;
        }
        let parent = dirname_of(&rel);
        if self.by_dir.contains_key(&parent) {
            Some(rel)
        } else {
            None
        }
    }

    fn dir_path(&self, rel_dir: &str) -> String {
        if rel_dir.is_empty() {
            self.dest_root.clone()
        } else {
            format!("{}\\{}", self.dest_root, rel_dir)
        }
    }

    fn dir_mtime(&self, rel_dir: &str) -> Option<i64> {
        std::fs::metadata(long_path(&self.dir_path(rel_dir)))
            .ok()
            .map(|m| mtime_ns(&m))
    }

    /// 扫描目标端一个目录，统计已完整 / 已最新的文件。
    fn scan_dir(&mut self, rel_dir: &str, mark_dirty: bool) {
        let Some(manifest) = self.by_dir.get(rel_dir) else {
            return;
        };
        let dir_path = self.dir_path(rel_dir);
        let mut done_bytes = 0u64;
        let mut done_files = 0u64;
        let mut fresh_bytes = 0u64;
        let mut fresh_files = 0u64;
        let mut in_flight = 0usize;

        match std::fs::read_dir(long_path(&dir_path)) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let Some(source) = manifest.get(&name) else {
                        continue;
                    };
                    let stat = match entry.metadata() {
                        Ok(s) if s.is_file() && !entry.file_type().map(|f| f.is_symlink()).unwrap_or(false) => s,
                        _ => continue,
                    };
                    if stat.len() != source.size {
                        in_flight += 1;
                        continue;
                    }
                    done_bytes += source.size;
                    done_files += 1;
                    if (mtime_ns(&stat) - source.mtime_ns).abs() <= self.tol_ns {
                        fresh_bytes += source.size;
                        fresh_files += 1;
                    }
                }
            }
            Err(_) => {
                done_bytes = 0;
                done_files = 0;
                fresh_bytes = 0;
                fresh_files = 0;
                in_flight = 0;
            }
        }

        self.done_sizes.insert(rel_dir.to_string(), done_bytes);
        self.done_counts.insert(rel_dir.to_string(), done_files);
        self.uptodate_sizes.insert(rel_dir.to_string(), fresh_bytes);
        self.uptodate_counts.insert(rel_dir.to_string(), fresh_files);
        self.scanned_dirs += 1;

        if mark_dirty && in_flight > 0 {
            // 有正在写入的文件：该目录的 mtime 在文件写完时不会变化，
            // 必须每轮都看一眼，否则进度会卡住
            self.dirty.insert(rel_dir.to_string());
        } else {
            self.dirty.remove(rel_dir);
        }
    }

    /// 把已经全部复制完的目录移出轮询集合。
    fn prune(&mut self) {
        self.active.retain(|rel_dir| {
            let expected = self.by_dir[rel_dir].len() as u64;
            self.done_counts.get(rel_dir).copied() != Some(expected)
        });
    }

    fn recompute(&mut self) {
        self.done_bytes = self.done_sizes.values().sum();
        self.done_files = self.done_counts.values().sum();
        self.uptodate_bytes = self.uptodate_sizes.values().sum();
        self.uptodate_files = self.uptodate_counts.values().sum();
    }
}

/// 目录是否为空（辅助断言，测试用）。
#[allow(dead_code)]
fn dir_is_empty(p: &Path) -> bool {
    std::fs::read_dir(p).map(|mut d| d.next().is_none()).unwrap_or(true)
}

// ---------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir_with(tag: &str) -> PathBuf {
        let base = std::env::temp_dir();
        let p = base.join(format!("rc_scanner_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_file(p: &Path, content: &str) {
        fs::write(p, content).unwrap();
    }

    #[test]
    fn scan_source_collects_files_dirs_and_bytes() {
        let src = temp_dir_with("scan");
        fs::create_dir_all(src.join("sub\\deep")).unwrap();
        write_file(&src.join("a.txt"), "aaaa");
        write_file(&src.join("sub\\b.bin"), "bbbbbbb");
        write_file(&src.join("sub\\deep\\c.dat"), "cc");

        let result = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        assert_eq!(result.file_count, 3);
        assert_eq!(result.total_bytes, 4 + 7 + 2);
        assert!(!result.cancelled);
        assert!(result.files.contains_key("a.txt"));
        assert!(result.files.contains_key("sub\\b.bin"));
        assert!(result.files.contains_key("sub\\deep\\c.dat"));
        assert!(result.dirs.contains(&String::new()));
        assert!(result.dirs.iter().any(|d| d == "sub\\deep"));
        fs::remove_dir_all(&src).ok();
    }

    #[test]
    fn scan_source_respects_excludes() {
        let src = temp_dir_with("excl");
        fs::create_dir_all(src.join("keep")).unwrap();
        fs::create_dir_all(src.join("skipme")).unwrap();
        write_file(&src.join("keep\\x.txt"), "xx");
        write_file(&src.join("skipme\\y.txt"), "yy");

        let result = scan_source(
            &src.to_string_lossy(),
            None,
            &[src.join("skipme").to_string_lossy().into_owned()],
            None,
            0.3,
        );
        assert_eq!(result.file_count, 1);
        assert_eq!(result.total_bytes, 2);
        fs::remove_dir_all(&src).ok();
    }

    #[test]
    fn scan_source_cancels() {
        let src = temp_dir_with("cancel");
        for i in 0..50 {
            fs::create_dir_all(src.join(format!("d{}", i))).unwrap();
            write_file(&src.join(format!("d{}\\f.txt", i)), "x");
        }
        let flag = AtomicBool::new(true);
        let result = scan_source(&src.to_string_lossy(), Some(&flag), &[], None, 0.3);
        assert!(result.cancelled);
        assert!(result.file_count < 50);
        fs::remove_dir_all(&src).ok();
    }

    #[test]
    fn tracker_baseline_and_poll() {
        let src = temp_dir_with("t_src");
        let dst = temp_dir_with("t_dst");

        // 源：两个目录各两个文件
        fs::create_dir_all(src.join("one")).unwrap();
        fs::create_dir_all(src.join("two")).unwrap();
        write_file(&src.join("one\\a.txt"), "aaaa");
        write_file(&src.join("one\\b.txt"), "bbb");
        write_file(&src.join("two\\c.txt"), "cc");

        let scan = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        let tracker = TargetProgressTracker::new(&scan, &dst.to_string_lossy());

        // 目标端：one/a.txt 已完整且最新
        fs::create_dir_all(dst.join("one")).unwrap();
        let a_meta = fs::metadata(src.join("one\\a.txt")).unwrap();
        let a_time = a_meta.modified().unwrap();
        fs::write(dst.join("one\\a.txt"), "aaaa").unwrap();
        let f = fs::File::options()
            .write(true)
            .open(dst.join("one\\a.txt"))
            .unwrap();
        f.set_modified(a_time).unwrap();
        drop(f);

        let baseline = tracker.build_baseline(None);
        assert_eq!(baseline, 4); // done = a.txt
        assert_eq!(tracker.uptodate_bytes(), 4); // mtime 一致 → uptodate

        // 目标端写完了 b.txt（mtime 不一致也算 done，不算 uptodate）
        fs::write(dst.join("one\\b.txt"), "bbb").unwrap();
        let after = tracker.poll();
        assert_eq!(after, 7);

        // 最终终扫
        fs::create_dir_all(dst.join("two")).unwrap();
        fs::write(dst.join("two\\c.txt"), "cc").unwrap();
        let final_bytes = tracker.final_poll();
        assert_eq!(final_bytes, 9);
        assert_eq!(tracker.done_files(), 3);

        // 全部完成后 prune 掉所有目录
        assert_eq!(tracker.active_dirs(), 0);
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn tracker_hint_with_relative_and_absolute_paths() {
        let src = temp_dir_with("h_src");
        let dst = temp_dir_with("h_dst");
        fs::create_dir_all(src.join("d")).unwrap();
        write_file(&src.join("d\\f.txt"), "x");

        let scan = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        let tracker = TargetProgressTracker::new(&scan, &dst.to_string_lossy());

        tracker.hint(&format!("{}\\d\\f.txt", dst.to_string_lossy()));
        tracker.hint(&format!("{}\\d\\f.txt", src.to_string_lossy()));
        tracker.hint("d\\f.txt");
        tracker.hint("somewhere\\else\\f.txt"); // 不在清单里，忽略

        // hint 后 poll 不 panic 且能把热点目录扫出来
        tracker.poll();
        assert_eq!(tracker.total_dirs(), 1);
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }
}
