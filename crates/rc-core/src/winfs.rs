//! Windows 文件系统工具：盘符枚举、容量查询、长路径处理与路径关系校验。
//!
//! 对应 Python 版 `core/winfs.py`。Windows 平台经 `windows-sys` 调用
//! kernel32（替代原 ctypes）；非 Windows 平台退化为空实现，保证模块可编译、可单测。

use serde::{Deserialize, Serialize};

pub const IS_WINDOWS: bool = cfg!(windows);

// ---------------------------------------------------------------------- 盘符信息

/// 一个已挂载的盘符信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveInfo {
    /// 形如 "E:\"
    pub root: String,
    pub label: String,
    pub fs: String,
    pub total: u64,
    pub free: u64,
}

impl DriveInfo {
    pub fn letter(&self) -> &str {
        &self.root[..2.min(self.root.len())]
    }

    pub fn display(&self) -> String {
        let name = if !self.label.is_empty() {
            self.label.clone()
        } else if self.total > 0 {
            "可移动磁盘".to_string()
        } else {
            "未知卷".to_string()
        };
        if self.total > 0 {
            format!("{}  {}  ·  可用 {}", self.letter(), name, human_bytes(self.free as f64))
        } else {
            format!("{}  {}", self.letter(), name)
        }
    }
}

#[cfg(windows)]
fn wstr_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 枚举本机所有已就绪盘符（含卷标、文件系统、容量、可用空间）。
#[cfg(windows)]
pub fn list_drives() -> Vec<DriveInfo> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetLogicalDrives, GetVolumeInformationW,
    };

    let mut out = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if (mask >> i) & 1 == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let rootw = to_wide(&root);

        let mut vol = [0u16; 261];
        let mut fsn = [0u16; 261];
        let mut serial = 0u32;
        let mut maxlen = 0u32;
        let mut flags = 0u32;
        let ok = unsafe {
            GetVolumeInformationW(
                rootw.as_ptr(),
                vol.as_mut_ptr(),
                261,
                &mut serial,
                &mut maxlen,
                &mut flags,
                fsn.as_mut_ptr(),
                261,
            )
        };
        let (label, fs) = if ok != 0 {
            (wstr_to_string(&vol), wstr_to_string(&fsn))
        } else {
            (String::new(), String::new())
        };

        let mut avail = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        let ok2 = unsafe {
            GetDiskFreeSpaceExW(rootw.as_ptr(), &mut avail, &mut total, &mut total_free)
        };
        if ok2 != 0 && total == 0 {
            // 无介质（空读卡器、未格式化卷）或不可访问，跳过
            continue;
        }
        out.push(DriveInfo {
            root,
            label,
            fs,
            total,
            free: avail,
        });
    }
    out
}

#[cfg(not(windows))]
pub fn list_drives() -> Vec<DriveInfo> {
    Vec::new()
}

/// 返回 (总容量, 可用容量)。失败时返回 (0, 0)。
pub fn disk_usage(path: &str) -> (u64, u64) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut avail = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        // 优先直接查询原始路径（GetDiskFreeSpaceExW 支持任意路径）
        let pw = to_wide(path);
        if unsafe { GetDiskFreeSpaceExW(pw.as_ptr(), &mut avail, &mut total, &mut total_free) } != 0 {
            return (total, avail);
        }
        let root = drive_root(path);
        if !root.is_empty() {
            let rw = to_wide(&root);
            if unsafe { GetDiskFreeSpaceExW(rw.as_ptr(), &mut avail, &mut total, &mut total_free) } != 0 {
                return (total, avail);
            }
        }
    }
    (0, 0)
}

/// 取出路径所属盘符根，如 "E:\a\b" -> "E:\"；UNC -> "\\\\server\\share\\"。
/// 非盘符路径返回 ""。
pub fn drive_root(path: &str) -> String {
    let abs = win_abspath(path);
    if abs.starts_with("\\\\") {
        // \\server\share\...
        let rest = abs.trim_start_matches('\\');
        let parts: Vec<&str> = rest.splitn(3, '\\').collect();
        if parts.len() >= 2 {
            format!("\\\\{}\\{}\\", parts[0], parts[1])
        } else {
            String::new()
        }
    } else if abs.len() >= 2 && abs.as_bytes()[1] == b':' {
        format!("{}\\", &abs[..2])
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------- 长路径

/// 转换为支持超过 MAX_PATH(260) 的扩展路径前缀形式。
///
/// 仅在需要时添加 `\\?\` 前缀；已带前缀或非 Windows 平台原样返回。
pub fn long_path(path: &str) -> String {
    if !IS_WINDOWS {
        return path.to_string();
    }
    if path.starts_with("\\\\?\\") {
        return path.to_string();
    }
    let absolute = win_abspath(path);
    if absolute.starts_with("\\\\") {
        return format!("\\\\?\\UNC\\{}", &absolute[2..]);
    }
    if absolute.len() < 240 {
        // 短路径不需要前缀，保持可读性（robocopy 日志里也更好看）
        return absolute;
    }
    format!("\\\\?\\{}", absolute)
}

/// 去掉扩展路径前缀，便于展示。
pub fn strip_long(path: &str) -> &str {
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        // 重新拼回 \\ 前缀：避免在返回 &str 时分配，走特殊分支
        let _ = rest;
        return path; // 由 strip_long_owned 处理需要分配的情况
    }
    if let Some(rest) = path.strip_prefix("\\\\?\\") {
        rest
    } else {
        path
    }
}

/// `strip_long` 的分配版本（UNC 前缀需要重新拼接）。
pub fn strip_long_owned(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{}", rest)
    } else if let Some(rest) = path.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

// ---------------------------------------------------------------------- 路径规范化

/// 词法级绝对化（不触碰文件系统），对应 `os.path.abspath` 语义。公开供
/// scanner / robocopy 等模块使用。
pub fn abspath(path: &str) -> String {
    win_abspath(path)
}

/// 词法级绝对化 + 分隔符归一（不触碰文件系统），对应 `os.path.abspath` +
/// `os.path.normpath` 的组合语义。
fn win_abspath(path: &str) -> String {
    let p = strip_long_owned(path);
    if p.starts_with("\\\\") || (p.len() >= 2 && p.as_bytes()[1] == b':') {
        normalize_seps(&p)
    } else {
        let cwd = std::env::current_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        normalize_seps(&format!("{}\\{}", cwd, p))
    }
}

/// 把 `/` 归一为 `\`，并词法消解 `.` 与 `..` 段。
/// 前缀三类：UNC(`\\…`)、扩展(`\\?\…`)、盘符(`X:`)。UNC 前两个段（server/share）
/// 不可被 `..` 弹出。
fn normalize_seps(p: &str) -> String {
    let p = p.replace('/', "\\");

    let (prefix, rest, min_keep): (&str, &str, usize) = if let Some(r) = p.strip_prefix("\\\\?\\UNC\\") {
        ("\\\\?\\UNC\\", r, 2)
    } else if let Some(r) = p.strip_prefix("\\\\?\\") {
        ("\\\\?\\", r, 0)
    } else if p.starts_with("\\\\") {
        ("\\\\", &p[2..], 2)
    } else if p.len() >= 2 && p.as_bytes()[1] == b':' {
        let (drive, r) = p.split_at(2);
        (drive, r.trim_start_matches('\\'), 0)
    } else {
        return p;
    };

    let mut stack: Vec<&str> = Vec::new();
    for seg in rest.split('\\') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.len() > min_keep {
                    stack.pop();
                }
            }
            s => stack.push(s),
        }
    }

    if prefix == "\\\\" || prefix == "\\\\?\\UNC\\" {
        format!("{}{}", prefix, stack.join("\\"))
    } else if prefix == "\\\\?\\" {
        format!("{}{}", prefix, stack.join("\\"))
    } else {
        // 盘符：E: + \a\b（无段时保持 "E:\"）
        if stack.is_empty() {
            format!("{}\\", prefix)
        } else {
            format!("{}\\{}", prefix, stack.join("\\"))
        }
    }
}

/// 规范化为可比较的绝对路径（统一大小写与分隔符），对应 `normcase(abspath)`。
pub fn norm_for_compare(path: &str) -> String {
    let abs = win_abspath(path);
    let mut s = abs.to_lowercase();
    while s.ends_with('\\') {
        s.pop();
    }
    if s.is_empty() {
        abs.to_lowercase()
    } else {
        s
    }
}

pub fn is_same_path(a: &str, b: &str) -> bool {
    norm_for_compare(a) == norm_for_compare(b)
}

/// 判断 child 是否位于 parent 内部（不含相等情况）。
pub fn is_subpath(child: &str, parent: &str) -> bool {
    let c = norm_for_compare(child);
    let p = norm_for_compare(parent);
    if c.is_empty() || p.is_empty() || c == p {
        return false;
    }
    let mut p = p;
    if !p.ends_with('\\') {
        p.push('\\');
    }
    c.starts_with(&p)
}

/// src 与 dst 的危险关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathRelation {
    /// 正常
    Ok,
    /// 源与目标相同
    Same,
    /// 目标位于源目录内部（会导致递归复制）
    DestInSrc,
    /// 源位于目标目录内部（同样危险）
    SrcInDest,
    /// 路径为空
    Empty,
    /// 源目录不存在
    Missing,
}

impl PathRelation {
    pub fn message(self) -> &'static str {
        match self {
            PathRelation::Ok => "",
            PathRelation::Empty => "源目录与目标目录都必须填写",
            PathRelation::Same => "源目录与目标目录不能相同",
            PathRelation::DestInSrc => "目标目录位于源目录内部，会导致自我递归复制，已阻止",
            PathRelation::SrcInDest => "源目录位于目标目录内部，可能造成数据覆盖，已阻止",
            PathRelation::Missing => "源目录不存在或不是文件夹",
        }
    }

    pub fn is_ok(self) -> bool {
        self == PathRelation::Ok
    }
}

/// 返回 src 与 dst 的危险关系。
pub fn path_relation(src: &str, dst: &str) -> PathRelation {
    if src.trim().is_empty() || dst.trim().is_empty() {
        return PathRelation::Empty;
    }
    if is_same_path(src, dst) {
        return PathRelation::Same;
    }
    if is_subpath(dst, src) {
        return PathRelation::DestInSrc;
    }
    if is_subpath(src, dst) {
        return PathRelation::SrcInDest;
    }
    if !std::path::Path::new(src).is_dir() {
        return PathRelation::Missing;
    }
    PathRelation::Ok
}

// ---------------------------------------------------------------------- 格式化

/// 字节数转人类可读文本。
pub fn human_bytes(size: f64) -> String {
    human_bytes_prec(size, 1)
}

pub fn human_bytes_prec(size: f64, precision: usize) -> String {
    let mut value = size;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if value.abs() < 1024.0 {
            if unit == "B" {
                return format!("{} B", value as i64);
            }
            return format!("{:.*} {}", precision, value, unit);
        }
        value /= 1024.0;
    }
    format!("{:.*} PB", precision, value)
}

/// 速度格式化，如 "123.4 MB/s"。
pub fn human_speed(bps: f64) -> String {
    if bps <= 0.0 {
        return "-- /s".to_string();
    }
    format!("{}/s", human_bytes(bps))
}

/// 时长格式化，如 "1 时 02 分" / "3 分 12 秒" / "45 秒"。
pub fn human_duration(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds else {
        return "--".to_string();
    };
    if seconds < 0.0 {
        return "--".to_string();
    }
    let total = seconds as u64;
    if total < 60 {
        return format!("{} 秒", total);
    }
    let (minutes, sec) = (total / 60, total % 60);
    if minutes < 60 {
        return format!("{} 分 {:02} 秒", minutes, sec);
    }
    let (hours, minute) = (minutes / 60, minutes % 60);
    format!("{} 时 {:02} 分", hours, minute)
}

// ---------------------------------------------------------------------- 其他

/// 定位 robocopy.exe。
pub fn robocopy_path() -> Option<String> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join("robocopy.exe");
            if cand.is_file() {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let cand = std::path::Path::new(&root)
        .join("System32")
        .join("Robocopy.exe");
    if cand.is_file() {
        Some(cand.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// 用资源管理器打开目录（对应 `os.startfile`）。
pub fn open_in_explorer(path: &str) -> bool {
    if !std::path::Path::new(path).exists() {
        return false;
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 检查目标盘空间是否足够，返回 (是否充足, 提示文本)。
pub fn free_space_hint(src_bytes: u64, dest_path: &str) -> (bool, String) {
    let (total, free) = disk_usage(dest_path);
    if total == 0 {
        return (true, "无法读取目标盘可用空间，请自行确认".to_string());
    }
    if src_bytes > free {
        (
            false,
            format!(
                "目标盘可用空间不足：需要 {}，可用仅 {}（总容量 {}）",
                human_bytes(src_bytes as f64),
                human_bytes(free as f64),
                human_bytes(total as f64)
            ),
        )
    } else {
        (
            true,
            format!(
                "目标盘可用 {}，本次约需 {}",
                human_bytes(free as f64),
                human_bytes(src_bytes as f64)
            ),
        )
    }
}

/// 当前 OEM 代码页（robocopy 控制台输出编码），非 Windows 平台返回 437。
#[cfg(windows)]
pub fn oem_codepage() -> u32 {
    unsafe { windows_sys::Win32::Globalization::GetOEMCP() }
}

#[cfg(not(windows))]
pub fn oem_codepage() -> u32 {
    437
}

// ---------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(0.0), "0 B");
        assert_eq!(human_bytes(512.0), "512 B");
        assert_eq!(human_bytes(1024.0), "1.0 KB");
        assert_eq!(human_bytes(1536.0), "1.5 KB");
        assert_eq!(human_bytes_prec(1024.0 * 1024.0, 2), "1.00 MB");
        assert_eq!(human_bytes(3.0 * 1024.0f64.powi(5)), "3.0 PB");
    }

    #[test]
    fn human_speed_and_duration() {
        assert_eq!(human_speed(0.0), "-- /s");
        assert_eq!(human_speed(1024.0 * 1024.0), "1.0 MB/s");
        assert_eq!(human_duration(None), "--");
        assert_eq!(human_duration(Some(-1.0)), "--");
        assert_eq!(human_duration(Some(45.0)), "45 秒");
        assert_eq!(human_duration(Some(75.0)), "1 分 15 秒");
        assert_eq!(human_duration(Some(3720.0)), "1 时 02 分");
    }

    #[test]
    fn strip_long_variants() {
        assert_eq!(strip_long_owned("\\\\?\\C:\\a\\b"), "C:\\a\\b");
        assert_eq!(strip_long_owned("\\\\?\\UNC\\srv\\share"), "\\\\srv\\share");
        assert_eq!(strip_long_owned("C:\\plain"), "C:\\plain");
        assert_eq!(strip_long("C:\\plain"), "C:\\plain");
    }

    #[test]
    fn long_path_adds_prefix_for_long_paths() {
        let long = format!("C:\\{}\\{}", "a".repeat(300), "b".repeat(50));
        let lp = long_path(&long);
        assert!(lp.starts_with("\\\\?\\C:\\"));
        assert_eq!(strip_long_owned(&lp), long);
        // 短路径不加前缀
        assert_eq!(long_path("C:\\short\\path"), "C:\\short\\path");
        // UNC 转扩展 UNC 前缀
        let unc = long_path("\\\\srv\\share\\dir");
        assert_eq!(unc, "\\\\?\\UNC\\srv\\share\\dir");
    }

    #[test]
    fn is_subpath_and_same() {
        assert!(is_subpath("C:\\a\\b\\c", "C:\\a"));
        // C:\a\b\..\a 规范化后是 C:\a\a，C:\a\a\b 位于其下
        assert!(is_subpath("C:\\a\\a\\b", "C:\\a\\b\\..\\a"));
        assert!(!is_subpath("C:\\a", "C:\\a"));
        assert!(!is_subpath("C:\\abc", "C:\\ab"));
        assert!(is_same_path("C:\\A\\B", "c:\\a\\b\\"));
        assert!(is_subpath("c:/a/b/c", "C:\\A"));
    }

    #[test]
    fn normalize_resolves_dotdot() {
        assert_eq!(normalize_seps("E:\\a\\b\\..\\c"), "E:\\a\\c");
        assert_eq!(normalize_seps("E:\\a\\.\\b"), "E:\\a\\b");
        assert_eq!(normalize_seps("E:\\.."), "E:\\");
        assert_eq!(normalize_seps("\\\\srv\\share\\x\\..\\y"), "\\\\srv\\share\\y");
        // UNC 的 server/share 不可被 .. 弹出
        assert_eq!(normalize_seps("\\\\srv\\share\\..\\y"), "\\\\srv\\share\\y");
        assert_eq!(normalize_seps("E:/a/b"), "E:\\a\\b");
    }

    #[test]
    fn drive_root_extraction() {
        assert_eq!(drive_root("E:\\a\\b"), "E:\\");
        assert_eq!(drive_root("E:/x"), "E:\\");
        assert_eq!(drive_root("\\\\srv\\share\\dir"), "\\\\srv\\share\\");
    }

    #[test]
    fn path_relation_cases() {
        let base = std::env::temp_dir();
        let src = base.join(format!("rc_test_src_{}", std::process::id()));
        std::fs::create_dir_all(&src).ok();
        let s = src.to_string_lossy().into_owned();

        assert_eq!(path_relation("", ""), PathRelation::Empty);
        assert_eq!(path_relation(&s, &s), PathRelation::Same);
        assert_eq!(path_relation(&s, &format!("{}\\sub", s)), PathRelation::DestInSrc);
        assert_eq!(
            path_relation(&format!("{}\\sub", s), &s),
            PathRelation::SrcInDest
        );
        assert_eq!(
            path_relation(&format!("{}\\nope", base.to_string_lossy()), "D:\\dst"),
            PathRelation::Missing
        );
        assert_eq!(path_relation(&s, "D:\\dst"), PathRelation::Ok);
        std::fs::remove_dir_all(&src).ok();
    }

    #[test]
    #[cfg(windows)]
    fn robocopy_locatable_on_windows() {
        assert!(robocopy_path().is_some());
    }
}
