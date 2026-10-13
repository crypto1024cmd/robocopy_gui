//! 复制后校验：不校验 / 大小+时间戳快速校验 / 哈希严格校验。
//!
//! 对应 Python 版 `core/verify.py`。
//!
//! 设计取舍：
//! - 快速校验只做 stat，成本极低，适合绝大多数场景（robocopy 自己复制时已校验过一遍）；
//! - 严格校验要读两遍文件内容，会与复制争抢磁盘 IO，因此只在任务复制结束后执行，
//!   并且用线程池并发（IO 密集，线程比进程更合适）；
//! - 大小不一致的文件直接判失败，不做无意义的哈希。

use crate::models::{HashAlgo, VerifyDiff, VerifyLevel};
use crate::scanner::{mtime_ns, ScanResult, SourceEntry};
use crate::winfs::{abspath, long_path};
use md5::Digest;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

/// 哈希分块大小：1MB 流式读取，禁止一次性 read() 大文件
pub const HASH_CHUNK: usize = 1024 * 1024;

/// 校验失败记录上限，避免差异列表海量刷屏
pub const MAX_FAILED_RECORDS: usize = 500;

/// 校验进度回调的最小间隔（秒）
pub const PROGRESS_INTERVAL: f64 = 0.2;

pub const DEFAULT_WORKERS: usize = 4;

/// 校验结果。
#[derive(Debug, Clone, Default)]
pub struct VerifyResult {
    pub level: VerifyLevel,
    pub algo: Option<HashAlgo>,
    pub checked: u64,
    pub passed: u64,
    pub failed: Vec<VerifyDiff>,
    pub failed_count: u64,
    pub cancelled: bool,
    pub elapsed: f64,
}

impl VerifyResult {
    pub fn ok(&self) -> bool {
        self.failed_count == 0 && !self.cancelled
    }
}

/// 分块计算文件哈希，避免大文件占满内存。
pub fn hash_file(path: &str, algo: HashAlgo, chunk_size: usize) -> std::io::Result<String> {
    let mut f = std::fs::File::open(long_path(path))?;
    match algo {
        HashAlgo::Md5 => hash_reader::<md5::Md5>(&mut f, chunk_size),
        HashAlgo::Sha1 => hash_reader::<sha1::Sha1>(&mut f, chunk_size),
        HashAlgo::Sha256 => hash_reader::<sha2::Sha256>(&mut f, chunk_size),
    }
}

fn hash_reader<D: Digest>(reader: &mut dyn Read, chunk_size: usize) -> std::io::Result<String> {
    let mut hasher = D::new();
    let mut buf = vec![0u8; chunk_size.max(4096)];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let out = hasher.finalize();
    Ok(out.iter().map(|b| format!("{:02x}", b)).collect())
}

fn record(result: &mut VerifyResult, rel: &str, reason: String) {
    result.failed_count += 1;
    if result.failed.len() < MAX_FAILED_RECORDS {
        result.failed.push(VerifyDiff {
            path: rel.to_string(),
            reason,
        });
    }
}

enum Outcome {
    Pass,
    Fail(String, String),
    Cancelled,
}

fn check_one(entry: &SourceEntry, src_root: &str, dest_root: &str, algo: HashAlgo) -> Outcome {
    let dest_path = format!("{}\\{}", dest_root, entry.rel);
    let src_path = format!("{}\\{}", src_root, entry.rel);
    let stat = match std::fs::metadata(long_path(&dest_path)) {
        Ok(s) => s,
        Err(exc) => {
            return Outcome::Fail(
                entry.rel.clone(),
                format!("无法读取目标文件（{}）", exc),
            )
        }
    };
    if stat.len() != entry.size {
        return Outcome::Fail(
            entry.rel.clone(),
            format!("大小不一致：目标 {} / 源 {}", stat.len(), entry.size),
        );
    }
    match (
        hash_file(&src_path, algo, HASH_CHUNK),
        hash_file(&dest_path, algo, HASH_CHUNK),
    ) {
        (Ok(a), Ok(b)) if a == b => Outcome::Pass,
        (Ok(_), Ok(_)) => Outcome::Fail(entry.rel.clone(), format!("{} 哈希不一致", algo.label())),
        (Err(exc), _) | (_, Err(exc)) => {
            Outcome::Fail(entry.rel.clone(), format!("读取失败（{}）", exc))
        }
    }
}

/// 按指定级别校验 scan 清单中的所有文件是否已正确到达目标目录。
#[allow(clippy::too_many_arguments)]
pub fn verify_task(
    scan: &ScanResult,
    dest_root: &str,
    level: VerifyLevel,
    algo: HashAlgo,
    progress_cb: Option<&mut dyn FnMut(usize, usize)>,
    cancel: Option<Arc<AtomicBool>>,
    workers: usize,
    mtime_tolerance_ns: i64,
) -> VerifyResult {
    let mut result = VerifyResult {
        level,
        algo: Some(algo),
        ..Default::default()
    };
    if level == VerifyLevel::None {
        return result;
    }

    let started = Instant::now();
    let items: Vec<&SourceEntry> = scan.files.values().collect();
    let total = items.len();
    let dest_root = abspath(dest_root);
    let mut last_report = Instant::now();
    let mut progress = progress_cb;

    let mut report = |force: bool, checked: usize| {
        if let Some(cb) = progress.as_mut() {
            if force || last_report.elapsed().as_secs_f64() >= PROGRESS_INTERVAL {
                last_report = Instant::now();
                cb(checked, total);
            }
        }
    };

    let cancelled = || cancel.as_deref().map(|f| f.load(Ordering::Relaxed)).unwrap_or(false);

    let mut checked: usize = 0;
    let mut passed: u64 = 0;

    // ---------------------------------------------------------- 快速校验
    if level == VerifyLevel::Quick {
        for entry in &items {
            if cancelled() {
                result.cancelled = true;
                break;
            }
            let dest_path = format!("{}\\{}", dest_root, entry.rel);
            match std::fs::metadata(long_path(&dest_path)) {
                Err(exc) => {
                    record(&mut result, &entry.rel, format!("无法读取目标文件（{}）", exc));
                }
                Ok(stat) => {
                    if stat.len() != entry.size {
                        record(
                            &mut result,
                            &entry.rel,
                            format!("大小不一致：目标 {} / 源 {}", stat.len(), entry.size),
                        );
                    } else if (mtime_ns(&stat) - entry.mtime_ns).abs() > mtime_tolerance_ns {
                        record(&mut result, &entry.rel, "修改时间不一致".to_string());
                    } else {
                        passed += 1;
                    }
                }
            }
            checked += 1;
            result.checked = checked as u64;
            result.passed = passed;
            report(false, checked);
        }
        report(true, checked);
        result.elapsed = started.elapsed().as_secs_f64();
        return result;
    }

    // ---------------------------------------------------------- 严格校验
    let pool_size = if workers == 0 {
        DEFAULT_WORKERS
    } else {
        workers.clamp(1, 8)
    };
    let (tx_item, rx_item) = mpsc::channel::<SourceEntry>();
    let (tx_out, rx_out) = mpsc::channel::<Outcome>();
    let rx_item = std::sync::Arc::new(std::sync::Mutex::new(rx_item));

    for _ in 0..pool_size {
        let rx = std::sync::Arc::clone(&rx_item);
        let tx = tx_out.clone();
        let dest_root = dest_root.clone();
        let src_root = scan.root.clone();
        let cancel = cancel.clone();
        std::thread::Builder::new()
            .name("verify".to_string())
            .spawn(move || loop {
                let entry = { rx.lock().unwrap().recv() };
                let Ok(entry) = entry else { break };
                if let Some(f) = cancel.as_deref() {
                    if f.load(Ordering::Relaxed) {
                        let _ = tx.send(Outcome::Cancelled);
                        continue;
                    }
                }
                let outcome = check_one(&entry, &src_root, &dest_root, algo);
                let _ = tx.send(outcome);
            })
            .ok();
    }
    for entry in &items {
        let _ = tx_item.send((*entry).clone());
    }
    drop(tx_item);

    let mut received = 0usize;
    while received < total {
        let Ok(outcome) = rx_out.recv() else { break };
        received += 1;
        if cancelled() {
            result.cancelled = true;
            break;
        }
        checked += 1;
        result.checked = checked as u64;
        match outcome {
            Outcome::Pass => {
                passed += 1;
                result.passed = passed;
            }
            Outcome::Cancelled => {
                result.cancelled = true;
                break;
            }
            Outcome::Fail(rel, reason) => record(&mut result, &rel, reason),
        }
        report(false, checked);
    }

    report(true, checked);
    result.elapsed = started.elapsed().as_secs_f64();
    result
}

// ---------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{scan_source, MTIME_TOLERANCE_NS};
    use std::fs;
    use std::path::PathBuf;

    fn temp_tree(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir();
        let src = base.join(format!("rc_verify_src_{}", tag));
        let dst = base.join(format!("rc_verify_dst_{}", tag));
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub\\b.txt"), "world!!").unwrap();
        (src, dst, base)
    }

    fn sync_mtime(src: &PathBuf, dst: &PathBuf, rel: &str) {
        let t = fs::metadata(src.join(rel)).unwrap().modified().unwrap();
        let f = fs::File::options()
            .write(true)
            .open(dst.join(rel))
            .unwrap();
        f.set_modified(t).unwrap();
    }

    fn copy_tree(src: &PathBuf, dst: &PathBuf) {
        fs::create_dir_all(dst.join("sub")).unwrap();
        fs::copy(src.join("a.txt"), dst.join("a.txt")).unwrap();
        fs::copy(src.join("sub\\b.txt"), dst.join("sub\\b.txt")).unwrap();
        sync_mtime(src, dst, "a.txt");
        sync_mtime(src, dst, "sub\\b.txt");
    }

    #[test]
    fn hash_file_known_vectors() {
        let base = std::env::temp_dir();
        let p = base.join(format!("rc_hash_{}.bin", std::process::id()));
        fs::write(&p, "abc").unwrap();
        let path = p.to_string_lossy().into_owned();
        assert_eq!(
            hash_file(&path, HashAlgo::Md5, HASH_CHUNK).unwrap(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hash_file(&path, HashAlgo::Sha1, HASH_CHUNK).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hash_file(&path, HashAlgo::Sha256, HASH_CHUNK).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fs::remove_file(&p).ok();
    }

    #[test]
    fn quick_verify_passes_identical_tree() {
        let (src, dst, _) = temp_tree("quick_ok");
        copy_tree(&src, &dst);
        let scan = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        let result = verify_task(
            &scan,
            &dst.to_string_lossy(),
            VerifyLevel::Quick,
            HashAlgo::Md5,
            None,
            None,
            DEFAULT_WORKERS,
            MTIME_TOLERANCE_NS,
        );
        assert_eq!(result.checked, 2);
        assert_eq!(result.passed, 2);
        assert!(result.ok());
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn quick_verify_flags_size_mismatch() {
        let (src, dst, _) = temp_tree("quick_bad");
        copy_tree(&src, &dst);
        fs::write(dst.join("a.txt"), "changed content").unwrap();
        let scan = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        let result = verify_task(
            &scan,
            &dst.to_string_lossy(),
            VerifyLevel::Quick,
            HashAlgo::Md5,
            None,
            None,
            DEFAULT_WORKERS,
            MTIME_TOLERANCE_NS,
        );
        assert_eq!(result.failed_count, 1);
        assert!(!result.ok());
        assert!(result.failed[0].reason.contains("大小不一致"));
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn quick_verify_flags_missing_file() {
        let (src, dst, _) = temp_tree("quick_missing");
        copy_tree(&src, &dst);
        fs::remove_file(dst.join("sub\\b.txt")).unwrap();
        let scan = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        let result = verify_task(
            &scan,
            &dst.to_string_lossy(),
            VerifyLevel::Quick,
            HashAlgo::Md5,
            None,
            None,
            DEFAULT_WORKERS,
            MTIME_TOLERANCE_NS,
        );
        assert_eq!(result.failed_count, 1);
        assert!(result.failed[0].reason.contains("无法读取目标文件"));
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn strict_verify_detects_content_corruption() {
        let (src, dst, _) = temp_tree("strict");
        copy_tree(&src, &dst);
        // 修改内容但保持大小一致：快速校验过，严格校验必须抓出来
        let orig = fs::read(dst.join("sub\\b.txt")).unwrap();
        let mut flipped = orig.clone();
        flipped[0] = if flipped[0] == b'w' { b'v' } else { b'w' };
        fs::write(dst.join("sub\\b.txt"), flipped).unwrap();
        sync_mtime(&src, &dst, "sub\\b.txt");

        let scan = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        let result = verify_task(
            &scan,
            &dst.to_string_lossy(),
            VerifyLevel::Strict,
            HashAlgo::Sha256,
            None,
            None,
            DEFAULT_WORKERS,
            MTIME_TOLERANCE_NS,
        );
        assert_eq!(result.checked, 2);
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed_count, 1);
        assert!(result.failed[0].reason.contains("SHA256 哈希不一致"));
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn none_level_skips_everything() {
        let (src, dst, _) = temp_tree("none");
        let scan = scan_source(&src.to_string_lossy(), None, &[], None, 0.3);
        let result = verify_task(
            &scan,
            &dst.to_string_lossy(),
            VerifyLevel::None,
            HashAlgo::Md5,
            None,
            None,
            DEFAULT_WORKERS,
            MTIME_TOLERANCE_NS,
        );
        assert_eq!(result.checked, 0);
        assert!(result.ok());
        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }
}
