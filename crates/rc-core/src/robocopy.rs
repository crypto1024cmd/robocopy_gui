//! robocopy 封装：命令构造、子进程执行、输出解码与退出码判定。
//!
//! 对应 Python 版 `core/robocopy.py`。
//!
//! 关键事实（决定了本模块的实现方式）：
//! 1. robocopy 的逐文件百分比进度条只在直接写控制台时输出，stdout 被重定向后
//!    就会消失，因此进度不能靠解析它的输出，只能靠 scanner 侧的目标目录轮询。
//!    这里仍然捕获 stdout，是为了在界面上展示逐文件事件与最终统计。
//! 2. 中文 Windows 下 robocopy 输出使用本地代码页（OEM CP，通常是 cp936），
//!    必须按其解码，否则日志乱码。
//! 3. 退出码 0-7 都是「成功/可接受」，只有 >= 8 才是真正的失败。

use crate::models::CopyOptions;
use crate::winfs;
use encoding_rs::Encoding;
use regex::Regex;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// 不弹出黑色控制台窗口
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------- 编码

/// robocopy 输出使用的编码（OEM 代码页 → encoding_rs 映射）。
pub fn console_encoding() -> &'static Encoding {
    use encoding_rs::*;
    match winfs::oem_codepage() {
        936 => GB18030,
        950 => BIG5,
        932 => SHIFT_JIS,
        949 => EUC_KR,
        1250 => WINDOWS_1250,
        1251 => WINDOWS_1251,
        1252 => WINDOWS_1252,
        1253 => WINDOWS_1253,
        1254 => WINDOWS_1254,
        1255 => WINDOWS_1255,
        1256 => WINDOWS_1256,
        1257 => WINDOWS_1257,
        866 => IBM866,
        _ => UTF_8,
    }
}

/// robocopy 输出编码名（日志头部展示用），如 "cp936"。
pub fn console_encoding_name() -> String {
    format!("cp{}", winfs::oem_codepage())
}

// ---------------------------------------------------------------------- 命令构造

/// 清理目录参数：去扩展前缀、去首尾空白、去尾部多余分隔符（盘符根除外）。
fn clean_dir(path: &str) -> String {
    let mut text = winfs::strip_long_owned(path.trim()).replace('/', "\\");
    if text.len() > 3 && text.ends_with('\\') {
        while text.ends_with('\\') {
            text.pop();
        }
    }
    text
}

/// 构造 robocopy 命令行（列表形式，引号转义由 [`list_to_command_line`] / OS 处理）。
///
/// 源可以是目录，也可以是**单个文件**：
/// - 目录：`robocopy <源> <目标>\<源目录名> /E|/MIR …`（把源目录整体建到目标里，
///   目标端出现与源同名的子目录）；
/// - 单文件：`robocopy <父目录> <目标> <文件名> …`（文件名作为过滤条件，文件直接放进
///   目标目录）；单文件没有递归 / 镜像语义，因此**不带** `/E` 与 `/MIR`。
///
/// 固定使用「安静模式 + 精确字节」参数，并默认加 /XJ 避免跟随联接点造成递归。
pub fn build_command(source: &str, dest: &str, options: &CopyOptions) -> Vec<String> {
    let opts = options.normalized();
    let exe = winfs::robocopy_path().unwrap_or_else(|| "robocopy".to_string());

    // 单文件模式：源用父目录 + 文件名作过滤条件
    if let Some((parent, file_name)) = winfs::split_file_source(source) {
        let mut cmd = vec![exe, clean_dir(&parent), clean_dir(dest), file_name];
        cmd.push("/XJ".into());
        cmd.push(format!("/MT:{}", opts.threads));
        cmd.push("/COPY:DAT".into());
        cmd.push(format!("/R:{}", opts.retries));
        cmd.push(format!("/W:{}", opts.wait_seconds));
        cmd.push("/BYTES".into());
        cmd.push("/NP".into());
        cmd.push("/NDL".into());
        if opts.restartable {
            cmd.push("/Z".into());
        }
        if opts.dry_run {
            cmd.push("/L".into());
        }
        return cmd;
    }

    // 目录模式：把源目录整体建到目标目录下（<目标>\<源目录名>）
    let target = winfs::effective_dest_root(source, dest);
    let mut cmd = vec![exe, clean_dir(source), clean_dir(&target)];

    // 复制范围
    cmd.push(if opts.mirror { "/MIR".into() } else { "/E".into() });
    // 排除联接点/符号链接，防止意外递归
    cmd.push("/XJ".into());

    // 提速与复制属性
    cmd.push(format!("/MT:{}", opts.threads));
    cmd.push("/COPY:DAT".into());

    // 失败重试策略：少量重试，避免坏文件卡死整批任务
    cmd.push(format!("/R:{}", opts.retries));
    cmd.push(format!("/W:{}", opts.wait_seconds));

    // 输出精简：/NP 去掉百分比、/NDL 去掉目录清单、/BYTES 让统计表输出精确字节。
    // 统计表本身要保留，任务结束后用它核对真实复制结果。
    cmd.push("/BYTES".into());
    cmd.push("/NP".into());
    cmd.push("/NDL".into());

    if opts.restartable {
        cmd.push("/Z".into());
    }
    if opts.dry_run {
        cmd.push("/L".into());
    }
    cmd
}

/// MSVCRT 规则的命令行渲染（等价 Python 的 `subprocess.list2cmdline`）。
pub fn list_to_command_line(cmd: &[String]) -> String {
    let mut out: Vec<String> = Vec::with_capacity(cmd.len());
    for arg in cmd {
        let needs_quotes = arg.is_empty()
            || arg.chars().any(|c| matches!(c, ' ' | '\t' | '"' | '\n'));
        let mut s = String::new();
        let mut backslashes = 0usize;
        for ch in arg.chars() {
            if ch == '"' {
                s.push_str(&"\\".repeat(backslashes * 2 + 1));
                s.push('"');
                backslashes = 0;
            } else {
                if ch == '\\' {
                    backslashes += 1;
                } else {
                    backslashes = 0;
                }
                s.push(ch);
            }
        }
        if needs_quotes {
            s = format!("\"{}{}\"", s, "\\".repeat(backslashes));
        }
        out.push(s);
    }
    out.join(" ")
}

/// 把命令行渲染成可直接粘贴到 cmd 的字符串。
pub fn preview_command(source: &str, dest: &str, options: &CopyOptions) -> String {
    list_to_command_line(&build_command(source, dest, options))
}

// ---------------------------------------------------------------------- 退出码

/// robocopy 语义：0-7 均视为成功/可接受，>= 8 才是失败。
pub fn is_success(returncode: i32) -> bool {
    (0..8).contains(&returncode)
}

fn rc_text(returncode: i32) -> Option<&'static str> {
    Some(match returncode {
        0 => "无文件需要复制（目标已是最新）",
        1 => "成功复制了文件",
        2 => "目标端存在额外的文件/目录",
        3 => "成功复制，且目标端存在额外的文件/目录",
        4 => "存在不匹配（部分文件未能正确复制）",
        5 => "存在不匹配，且已复制部分文件",
        6 => "存在不匹配，且目标端有额外文件",
        7 => "存在不匹配与额外文件，且已复制部分文件",
        8 => "复制失败：有文件无法复制",
        16 => "严重错误：robocopy 未能正常启动或参数非法",
        _ => return None,
    })
}

pub fn exit_code_text(returncode: i32) -> String {
    if returncode < 0 {
        return "进程被中止".to_string();
    }
    if let Some(text) = rc_text(returncode) {
        return format!("[{}] {}", returncode, text);
    }
    if returncode >= 16 {
        return format!("[{}] 严重错误，请查看日志", returncode);
    }
    format!("[{}] 未知退出码", returncode)
}

// ---------------------------------------------------------------------- 输出解析

/// robocopy 输出中的一行文件/目录事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLine {
    pub action: String,
    pub size: u64,
    pub path: String,
}

impl FileLine {
    pub fn is_file(&self) -> bool {
        !self.action.to_lowercase().contains("dir")
    }
}

fn file_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?P<action>\*?EXTRA\s+(?:Dir|File)|New\s+Dir|New\s+File|Newer|Older|Changed|same)\s+(?P<size>[\d,]+)?\s*(?P<path>\S.*?)\s*$",
        )
        .expect("robocopy 文件行正则不合法")
    })
}

/// 解析 robocopy 的逐文件输出行，仅用于界面展示，失败即返回 None。
pub fn parse_file_line(line: &str) -> Option<FileLine> {
    let m = file_line_re().captures(line)?;
    let action = m
        .name("action")?
        .as_str()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let raw_size = m
        .name("size")
        .map(|s| s.as_str())
        .unwrap_or("0")
        .replace(',', "");
    let size: u64 = raw_size.parse().unwrap_or(0);
    let path = winfs::strip_long_owned(m.name("path")?.as_str().trim());
    if path.is_empty() {
        return None;
    }
    Some(FileLine { action, size, path })
}

/// 解析统计表里的字节数（/BYTES 下是纯数字，兼容 k/m/g 后缀写法）。
fn parse_size_token(token: &str) -> u64 {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^([\d.]+)\s*([kmgtep]?)").expect("size 正则不合法"));
    let token: String = token.trim().to_lowercase().replace(',', "");
    let Some(m) = re.captures(&token) else {
        return 0;
    };
    let value: f64 = m.get(1).unwrap().as_str().parse().unwrap_or(0.0);
    let unit = m.get(2).map(|x| x.as_str()).unwrap_or("");
    let scale: f64 = match unit {
        "" => 1.0,
        "k" => 1024.0,
        "m" => 1024.0f64.powi(2),
        "g" => 1024.0f64.powi(3),
        "t" => 1024.0f64.powi(4),
        "p" => 1024.0f64.powi(5),
        _ => 1.0,
    };
    (value * scale) as u64
}

/// robocopy 统计表解析结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CopySummary {
    pub dirs_total: u64,
    pub dirs_copied: u64,
    pub files_total: u64,
    pub files_copied: u64,
    pub files_skipped: u64,
    pub files_failed: u64,
    pub files_extra: u64,
    pub bytes_total: u64,
    pub bytes_copied: u64,
}

pub fn empty_summary() -> CopySummary {
    CopySummary::default()
}

/// 解析 robocopy 统计表文本（日志尾部刚读出来的那段）。
///
/// 同时支持英文（Dirs/Files/Bytes）与中文（目录/文件/字节）列名：
/// 原 Python 版只匹配英文，导致中文 Windows 下统计表永远解析失败，
/// 这里按「对齐功能、不对齐缺陷」补齐中文列名（含全角冒号）。
pub fn parse_summary_text(text: &str) -> CopySummary {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?im)^\s*(Dirs|Files|Bytes|目录|文件|字节)\s*[:：]\s*(.+)$")
            .expect("summary 正则不合法")
    });

    let mut result = CopySummary::default();
    for m in re.captures_iter(text) {
        let raw = m.get(1).unwrap().as_str();
        let name = match raw {
            "目录" => "dirs".to_string(),
            "文件" => "files".to_string(),
            "字节" => "bytes".to_string(),
            other => other.to_lowercase(),
        };
        let rest = m.get(2).unwrap().as_str();
        let tokens: Vec<&str> = rest.split_whitespace().take(6).collect();
        if tokens.len() < 2 {
            continue;
        }
        let mut values = [0u64; 6];
        if name == "bytes" {
            for (i, t) in tokens.iter().enumerate() {
                values[i] = parse_size_token(t);
            }
        } else {
            for (i, t) in tokens.iter().enumerate() {
                let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
                values[i] = digits.parse().unwrap_or(0);
            }
        }
        match name.as_str() {
            "dirs" => {
                result.dirs_total = values[0];
                result.dirs_copied = values[1];
            }
            "files" => {
                result.files_total = values[0];
                result.files_copied = values[1];
                result.files_skipped = values[2];
                result.files_failed = values[4];
                result.files_extra = values[5];
            }
            "bytes" => {
                result.bytes_total = values[0];
                result.bytes_copied = values[1];
            }
            _ => {}
        }
    }
    result
}

/// 从日志文件尾部（32KB 滑窗）解析 robocopy 统计表。
pub fn parse_summary(log_path: &str) -> CopySummary {
    use std::io::{Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(winfs::long_path(log_path)) else {
        return empty_summary();
    };
    let Ok(size) = f.seek(SeekFrom::End(0)) else {
        return empty_summary();
    };
    let start = size.saturating_sub(32 * 1024);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return empty_summary();
    }
    let mut tail = Vec::new();
    if f.read_to_end(&mut tail).is_err() {
        return empty_summary();
    }
    parse_summary_text(&String::from_utf8_lossy(&tail))
}

// ---------------------------------------------------------------------- 进程执行

/// 按 PID 终止外部 robocopy 进程（跨线程终止 worker 持有的子进程）。
#[cfg(windows)]
pub fn terminate_pid(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let ok = TerminateProcess(handle, 1) != 0;
        CloseHandle(handle);
        ok
    }
}

#[cfg(not(windows))]
pub fn terminate_pid(_pid: u32) -> bool {
    false
}

/// 执行 robocopy。
///
/// - `cmd`         [`build_command`] 的返回值
/// - `log_path`    日志落盘路径（UTF-8 文本），可为 None
/// - `on_line`     每读取到一行输出时回调 `on_line(line)`
/// - `stop_event`  外部停止信号；置位后本函数不再继续阻塞读取
/// - `register`    进程 PID 注册回调，便于外部随时按 PID 终止
/// - 返回          robocopy 退出码（进程被强杀时为 -1）；启动失败返回 Err
#[allow(clippy::too_many_arguments)]
pub fn run_robocopy(
    cmd: &[String],
    log_path: Option<&str>,
    mut on_line: Option<&mut dyn FnMut(&str)>,
    stop_event: Option<&AtomicBool>,
    register: Option<&dyn Fn(u32)>,
    encoding: Option<&'static Encoding>,
) -> std::io::Result<i32> {
    let encoding = encoding.unwrap_or_else(console_encoding);
    let mut decoder = encoding.new_decoder();

    // 日志打开失败时降级为「不落盘」，与 Python 版一致
    let mut log_file: Option<std::fs::File> = log_path.and_then(|p| {
        let dir = std::path::Path::new(p).parent().map(|d| d.to_path_buf());
        if let Some(d) = dir {
            std::fs::create_dir_all(&d).ok()?;
        }
        std::fs::File::create(winfs::long_path(p)).ok()
    });

    let mut command = std::process::Command::new(&cmd[0]);
    command
        .args(&cmd[1..])
        .stdout(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(exc) => {
            if let Some(f) = log_file.as_mut() {
                let _ = writeln!(f, "无法启动 robocopy: {}", exc);
            }
            return Err(exc);
        }
    };

    if let Some(cb) = register {
        cb(child.id());
    }

    let mut stdout = child.stdout.take();
    let mut buffer = String::new();
    'read: loop {
        let Some(pipe) = stdout.as_mut() else {
            break;
        };
        let mut chunk = [0u8; 8192];
        let n = match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        let mut text = String::new();
        // encoding_rs Decoder 无 Cow 版 decode，用 decode_to_str 写入 NUL 填充的
        // 合法 UTF-8 缓冲（多字节字符可安全跨越 8KB 读取边界）
        let need = decoder.max_utf8_buffer_length(n).unwrap_or(4 * n + 16);
        let mut out_bytes = vec![0u8; need];
        let out_str: &mut str = match std::str::from_utf8_mut(&mut out_bytes) {
            Ok(s) => s,
            Err(_) => unreachable!("NUL 填充必然是合法 UTF-8"),
        };
        let (_, _, written, _) = decoder.decode_to_str(&chunk[..n], out_str, false);
        text.push_str(&out_str[..written]);
        if let Some(f) = log_file.as_mut() {
            let _ = f.write_all(text.as_bytes());
        }
        if let Some(cb) = on_line.as_mut() {
            buffer.push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
            // 只把「完整行」推给回调，最后一段不完整行留在 buffer 等下一块
            let mut lines: Vec<&str> = buffer.split('\n').collect();
            let rest = lines.pop().unwrap_or("").to_string();
            for line in lines {
                let line = line.trim_end();
                if !line.trim().is_empty() {
                    cb(line);
                }
            }
            buffer.clear();
            buffer.push_str(&rest);
        }
        if let Some(flag) = stop_event {
            if flag.load(Ordering::Relaxed) {
                break 'read;
            }
        }
    }

    // 冲刷解码器剩余字节 + 缓冲中的最后一行
    let need = decoder.max_utf8_buffer_length(0).unwrap_or(16);
    let mut out_bytes = vec![0u8; need];
    let out_str: &mut str = match std::str::from_utf8_mut(&mut out_bytes) {
        Ok(s) => s,
        Err(_) => unreachable!("NUL 填充必然是合法 UTF-8"),
    };
    let (_, _, written, _) = decoder.decode_to_str(b"", out_str, true);
    let tail = format!("{}{}", &out_str[..written], buffer);
    if !tail.trim().is_empty() {
        if let Some(f) = log_file.as_mut() {
            let _ = f.write_all(tail.as_bytes());
        }
        if let Some(cb) = on_line.as_mut() {
            for line in tail.split('\n') {
                if !line.trim().is_empty() {
                    cb(line.trim_end());
                }
            }
        }
    }

    drop(stdout);

    // 最多等 10 秒让进程退出；超时强杀并返回 -1
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut returncode = -1;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                returncode = status.code().unwrap_or(-1);
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            returncode = -1;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if let Some(f) = log_file.as_mut() {
        let _ = f.flush();
    }
    Ok(returncode)
}

// ---------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_default_flags() {
        let cmd = build_command("C:\\src", "D:\\dst", &CopyOptions::default());
        // [0]=exe, [1]=源, [2]=目标, 之后是参数
        let exe = &cmd[0];
        assert!(
            exe.ends_with("robocopy.exe") || exe.eq_ignore_ascii_case("robocopy"),
            "exe = {}",
            exe
        );
        assert_eq!(cmd[1], "C:\\src");
        // 目录源整体建到目标下：目标端为 D:\dst\src
        assert_eq!(cmd[2], "D:\\dst\\src");
        assert!(cmd.contains(&"/E".to_string()));
        assert!(cmd.contains(&"/XJ".to_string()));
        assert!(cmd.contains(&"/MT:16".to_string()));
        assert!(cmd.contains(&"/COPY:DAT".to_string()));
        assert!(cmd.contains(&"/R:1".to_string()));
        assert!(cmd.contains(&"/W:1".to_string()));
        assert!(cmd.contains(&"/BYTES".to_string()));
        assert!(cmd.contains(&"/NP".to_string()));
        assert!(cmd.contains(&"/NDL".to_string()));
        assert!(!cmd.contains(&"/MIR".to_string()));
        assert!(!cmd.contains(&"/L".to_string()));
        assert!(!cmd.contains(&"/Z".to_string()));
    }

    #[test]
    fn build_command_optional_flags() {
        let mut o = CopyOptions::default();
        o.mirror = true;
        o.dry_run = true;
        o.restartable = true;
        let cmd = build_command("C:\\src", "D:\\dst", &o);
        assert!(cmd.contains(&"/MIR".to_string()));
        assert!(cmd.contains(&"/L".to_string()));
        assert!(cmd.contains(&"/Z".to_string()));
        assert!(!cmd.contains(&"/E".to_string()));
    }

    #[test]
    fn build_command_dir_source_creates_named_subdir() {
        let cmd = build_command("D:\\photos\\2024", "E:\\backup", &CopyOptions::default());
        assert_eq!(cmd[1], "D:\\photos\\2024");
        assert_eq!(cmd[2], "E:\\backup\\2024");
        assert!(cmd.contains(&"/E".to_string()));

        // 盘符根作源（无目录名）：目标目录保持原样
        let root = build_command("D:\\", "E:\\backup", &CopyOptions::default());
        assert_eq!(root[2], "E:\\backup");
    }

    #[test]
    fn build_command_clamps_and_cleans_paths() {
        let mut o = CopyOptions::default();
        o.threads = 999;
        o.retries = 99;
        o.wait_seconds = 999;
        let cmd = build_command("C:/a b/x/", "D:\\dst", &o);
        assert_eq!(cmd[1], "C:\\a b\\x"); // / → \，去尾分隔符
        assert_eq!(cmd[2], "D:\\dst\\x"); // 目录源：目标下新建同名子目录
        assert!(cmd.contains(&"/MT:128".to_string()));
        assert!(cmd.contains(&"/R:20".to_string()));
        assert!(cmd.contains(&"/W:120".to_string()));
    }

    #[test]
    fn list_to_command_line_quotes_spaces() {
        let cmd = vec!["rc.exe".to_string(), "C:\\a b\\c".to_string()];
        assert_eq!(list_to_command_line(&cmd), "rc.exe \"C:\\a b\\c\"");
        let plain = vec!["rc.exe".to_string(), "C:\\a".to_string()];
        assert_eq!(list_to_command_line(&plain), "rc.exe C:\\a");
    }

    #[test]
    fn preview_command_is_renderable() {
        let p = preview_command("C:\\my data", "D:\\dst", &CopyOptions::default());
        assert!(p.starts_with('"') || !p.contains(' ') || p.contains("\"C:\\my data\""));
        assert!(p.contains("robocopy"));
        assert!(p.contains("/E"));
    }

    #[test]
    fn build_command_single_file_uses_parent_and_filename() {
        let base = std::env::temp_dir();
        let dir = base.join(format!("rc_cmd_file_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a b.txt");
        std::fs::write(&file, "x").unwrap();

        // 即便用户勾了镜像，单文件也不得带 /MIR（没有镜像 / 递归语义）
        let opts = CopyOptions {
            mirror: true,
            ..Default::default()
        };
        let cmd = build_command(&file.to_string_lossy(), "D:\\out", &opts);

        // [0]=exe, [1]=父目录, [2]=目标, [3]=文件名过滤条件
        assert_eq!(
            cmd[1].to_lowercase(),
            dir.to_string_lossy().to_lowercase()
        );
        assert_eq!(cmd[2], "D:\\out");
        assert_eq!(cmd[3], "a b.txt");
        assert!(
            !cmd.iter().any(|a| a == "/E" || a == "/MIR"),
            "单文件不应带 /E 或 /MIR：{:?}",
            cmd
        );
        // 含空格的文件名渲染成可粘贴命令行时必须加引号
        let line = list_to_command_line(&cmd);
        assert!(line.contains("\"a b.txt\""), "{}", line);

        // 目录源仍走原有 /E 模式，不受影响
        let dir_cmd = build_command(&dir.to_string_lossy(), "D:\\out", &CopyOptions::default());
        assert!(dir_cmd.contains(&"/E".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exit_code_semantics() {
        assert!(is_success(0));
        assert!(is_success(7));
        assert!(!is_success(8));
        assert!(!is_success(16));
        assert_eq!(exit_code_text(-1), "进程被中止");
        assert_eq!(
            exit_code_text(0),
            "[0] 无文件需要复制（目标已是最新）"
        );
        assert_eq!(exit_code_text(16), "[16] 严重错误：robocopy 未能正常启动或参数非法");
        assert_eq!(exit_code_text(9), "[9] 未知退出码");
    }

    #[test]
    fn parse_file_line_variants() {
        let f = parse_file_line("        New File          1,234  D:\\data\\file.txt");
        assert!(f.is_some());
        let f = f.unwrap();
        assert_eq!(f.action, "New File");
        assert_eq!(f.size, 1234);
        assert_eq!(f.path, "D:\\data\\file.txt");
        assert!(f.is_file());

        let d = parse_file_line("      *EXTRA Dir          D:\\extra");
        let d = d.unwrap();
        assert!(!d.is_file());
        assert_eq!(d.action, "*EXTRA Dir");

        assert!(parse_file_line("   some random line").is_none());
        assert!(parse_file_line("").is_none());
    }

    #[test]
    fn parse_summary_text_english_table() {
        let text = "\n                Dirs:    12     3     0     0     9     0\n\
                    \n                Files:     5     2     1     0     1     0\n\
                    \n                Bytes:   1234   500     0     0   734     0\n";
        let s = parse_summary_text(text);
        assert_eq!(s.dirs_total, 12);
        assert_eq!(s.dirs_copied, 3);
        // Files 行六列：Total Copied Skipped Mismatch FAILED Extras
        assert_eq!(s.files_total, 5);
        assert_eq!(s.files_copied, 2);
        assert_eq!(s.files_skipped, 1);
        assert_eq!(s.files_failed, 1);
        assert_eq!(s.files_extra, 0);
        assert_eq!(s.bytes_total, 1234);
        assert_eq!(s.bytes_copied, 500);
    }

    #[test]
    fn parse_summary_text_chinese_table() {
        // 中文 Windows 下 robocopy 的统计表列名（原版在中文环境解析不到，此处补齐）
        let text = "\r\n                  总计        复制        跳过       不匹配       失败        其他\r\n\
                    \r\n   目录:        12     3     0     0     9     0\r\n\
                    \r\n   文件:         5     2     1     0     1     0\r\n\
                    \r\n   字节:      1234   500     0     0   734     0\r\n";
        let s = parse_summary_text(text);
        assert_eq!(s.dirs_total, 12);
        assert_eq!(s.dirs_copied, 3);
        assert_eq!(s.files_total, 5);
        assert_eq!(s.files_copied, 2);
        assert_eq!(s.files_skipped, 1);
        assert_eq!(s.bytes_total, 1234);
        assert_eq!(s.bytes_copied, 500);
    }

    #[test]
    fn parse_summary_text_handles_kmg_suffixes() {
        let s = parse_summary_text("Bytes:  1.5k  2m  0  0  0  0");
        assert_eq!(s.bytes_total, 1536);
        assert_eq!(s.bytes_copied, 2 * 1024 * 1024);
    }

    #[test]
    fn parse_summary_reads_log_tail() {
        let base = std::env::temp_dir();
        let log = base.join(format!("rc_test_log_{}.txt", std::process::id()));
        let header = "garbage\n".repeat(6000);
        let tail = "\n                Dirs:     1     1     0     0     0     0\n\
                    \n                Files:     2     2     0     0     0     0\n\
                    \n                Bytes:    10    10     0     0     0     0\n";
        std::fs::write(&log, format!("{}{}", header, tail)).unwrap();
        let s = parse_summary(&log.to_string_lossy());
        assert_eq!(s.files_total, 2);
        assert_eq!(s.bytes_copied, 10);
        std::fs::remove_file(&log).ok();
    }

    #[test]
    fn run_robocopy_captures_output_and_log() {
        // 用 cmd 模拟 robocopy 输出：验证管道读取、日志落盘与行回调
        let base = std::env::temp_dir();
        let log = base.join(format!("rc_test_run_{}.log", std::process::id()));
        let log_path = log.to_string_lossy().into_owned();

        let cmd = vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo hello robocopy".to_string(),
        ];
        let mut lines = Vec::new();
        let pid_holder = std::sync::Mutex::new(0u32);
        {
            let sink = std::sync::Mutex::new(&mut lines);
            let reg = |pid: u32| *pid_holder.lock().unwrap() = pid;
            let rc = run_robocopy(
                &cmd,
                Some(&log_path),
                Some(&mut |line: &str| sink.lock().unwrap().push(line.to_string())),
                None,
                Some(&reg),
                None,
            )
            .unwrap();
            assert_eq!(rc, 0);
        }
        assert!(*pid_holder.lock().unwrap() > 0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "hello robocopy");

        let content = std::fs::read_to_string(&log).unwrap();
        assert!(content.contains("hello robocopy"));
        std::fs::remove_file(&log).ok();
    }
}
