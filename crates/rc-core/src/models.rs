//! 数据模型：复制任务、复制选项、状态枚举与引擎事件。
//!
//! 对应 Python 版 `core/models.py`。本模块属于 core 层，不依赖任何 UI
//! 框架，是前端与引擎之间的数据契约（经 serde 序列化为 JSON）。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 单条日志行的最大字符数，超过截断并追加 `…`，防止超长路径卡死 UI。
pub const MAX_LINE_CHARS: usize = 400;

/// 当前 Unix 时间戳（秒，f64），对应 Python 版的 `time.time()`。
pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// 生成 8 位十六进制任务 ID，对应 Python 版 `uuid.uuid4().hex[:8]`。
/// 无第三方依赖：时间戳 + 原子计数器 + 进程号做 splitmix 混合。
fn new_task_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed) as u64;
    let pid = std::process::id() as u64;
    let mut h = t ^ c.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ pid.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    format!("{:08x}", (h & 0xFFFF_FFFF) as u32)
}

// ---------------------------------------------------------------------- 枚举

/// 任务生命周期状态。`label` 同时用作界面展示文案。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    #[default]
    Pending,
    Scanning,
    Running,
    Verifying,
    Done,
    Stopped,
    Failed,
}

impl TaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Pending => "待执行",
            TaskStatus::Scanning => "扫描中",
            TaskStatus::Running => "复制中",
            TaskStatus::Verifying => "校验中",
            TaskStatus::Done => "已完成",
            TaskStatus::Stopped => "已停止",
            TaskStatus::Failed => "失败",
        }
    }

    /// 是否为终态（终态任务不会再次被执行）。
    pub fn is_final(self) -> bool {
        matches!(
            self,
            TaskStatus::Done | TaskStatus::Stopped | TaskStatus::Failed
        )
    }
}

/// 复制后校验级别。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyLevel {
    #[default]
    None,
    Quick,
    Strict,
}

impl VerifyLevel {
    pub fn label(self) -> &'static str {
        match self {
            VerifyLevel::None => "不校验",
            VerifyLevel::Quick => "大小+时间戳",
            VerifyLevel::Strict => "哈希校验",
        }
    }
}

/// 严格校验使用的哈希算法。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgo {
    #[default]
    Md5,
    Sha1,
    Sha256,
}

impl HashAlgo {
    pub fn label(self) -> &'static str {
        match self {
            HashAlgo::Md5 => "MD5",
            HashAlgo::Sha1 => "SHA1",
            HashAlgo::Sha256 => "SHA256",
        }
    }
}

// ---------------------------------------------------------------------- 选项

/// 单个任务的复制参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyOptions {
    /// robocopy 多线程数量（/MT），1..128，是提速的主要手段
    pub threads: i32,
    /// true 使用 /MIR（镜像，会删除目标端多余文件）；false 使用 /E
    pub mirror: bool,
    /// true 使用 /L，只列出将要复制的文件而不写入
    pub dry_run: bool,
    pub verify_level: VerifyLevel,
    pub hash_algo: HashAlgo,
    /// 重试次数（/R），0..20
    pub retries: i32,
    /// 重试间隔秒数（/W），0..120
    pub wait_seconds: i32,
    /// true 使用 /Z 断点重启模式（更抗中断但明显降速）
    pub restartable: bool,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            threads: 16,
            mirror: false,
            dry_run: false,
            verify_level: VerifyLevel::Quick,
            hash_algo: HashAlgo::Md5,
            retries: 1,
            wait_seconds: 1,
            restartable: false,
        }
    }
}

impl CopyOptions {
    /// 夹取到合法范围，避免拼出非法的命令行参数。
    pub fn normalized(&self) -> CopyOptions {
        let mut c = self.clone();
        // Python 版语义：threads/retries/wait 为 0 时回退默认值再夹取
        c.threads = if c.threads == 0 {
            16
        } else {
            c.threads.clamp(1, 128)
        };
        c.retries = c.retries.clamp(0, 20);
        c.wait_seconds = c.wait_seconds.clamp(0, 120);
        c
    }

    /// 一行式摘要，用于界面提示与日志。
    pub fn summary_text(&self) -> String {
        let mut mode = if self.mirror { "镜像 /MIR" } else { "普通 /E" }.to_string();
        if self.dry_run {
            mode += " · 试运行 /L";
        }
        if self.restartable {
            mode += " · 重启模式 /Z";
        }
        format!("{} · /MT:{} · 校验 {}", mode, self.threads, self.verify_level.label())
    }
}

// ---------------------------------------------------------------------- 任务

/// 单条校验差异（对应 Python 版 `verify_failed` 的 (路径, 原因) 元组）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyDiff {
    pub path: String,
    pub reason: String,
}

/// 一个「源目录 → 目标目录」复制任务。
///
/// 除配置字段外，其余字段均为运行期状态，由 `engine` 在 worker 线程中更新。
/// UI 不直接持有本对象，而是通过 `TaskEvent` 拿到副本，避免跨线程竞争。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyTask {
    // ---- 配置 ----
    pub source: String,
    pub dest: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub options: CopyOptions,
    #[serde(default = "new_task_id")]
    pub task_id: String,

    // ---- 运行期状态 ----
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub phase_text: String,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub copied_bytes: u64,
    #[serde(default)]
    pub total_files: u64,
    #[serde(default)]
    pub copied_files: u64,
    #[serde(default)]
    pub already_bytes: u64,
    #[serde(default)]
    pub speed_bps: f64,
    #[serde(default)]
    pub avg_speed_bps: f64,
    #[serde(default)]
    pub eta_seconds: Option<f64>,
    #[serde(default)]
    pub current_file: String,
    #[serde(default)]
    pub returncode: Option<i32>,
    #[serde(default)]
    pub rc_text: String,
    #[serde(default)]
    pub log_path: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub verify_checked: u64,
    #[serde(default)]
    pub verify_passed: u64,
    #[serde(default)]
    pub verify_failed: Vec<VerifyDiff>,
    #[serde(default)]
    pub skipped_count: u64,
    #[serde(default)]
    pub started_at: f64,
    #[serde(default)]
    pub finished_at: f64,
    #[serde(default)]
    pub elapsed: f64,
}

impl CopyTask {
    pub fn new(source: impl Into<String>, dest: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            dest: dest.into(),
            ..Self::default()
        }
    }

    /// 完成百分比（0..100）。总字节未知时：终态且无错误给 100，否则给 0。
    pub fn percent(&self) -> f64 {
        if self.total_bytes == 0 {
            return if self.status.is_final() && self.error.is_empty() {
                100.0
            } else {
                0.0
            };
        }
        (self.copied_bytes as f64 * 100.0 / self.total_bytes as f64).clamp(0.0, 100.0)
    }

    pub fn remaining_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.copied_bytes)
    }

    pub fn display_name(&self) -> &str {
        if !self.name.is_empty() {
            return &self.name;
        }
        let trimmed = self.source.trim_end_matches(['\\', '/']);
        let tail = trimmed.rsplit(['\\', '/']).next().unwrap_or("");
        if tail.is_empty() {
            &self.source
        } else {
            tail
        }
    }

    pub fn route_text(&self) -> String {
        format!("{}  →  {}", self.source, self.dest)
    }

    /// 刷新耗时字段；结束时点由 `finished_at` 决定，否则取当前时刻。
    pub fn touch_elapsed(&mut self) {
        let end = if self.finished_at > 0.0 {
            self.finished_at
        } else {
            now_secs()
        };
        self.elapsed = if self.started_at > 0.0 {
            (end - self.started_at).max(0.0)
        } else {
            0.0
        };
    }
}

impl Default for CopyTask {
    fn default() -> Self {
        Self {
            source: String::new(),
            dest: String::new(),
            name: String::new(),
            options: CopyOptions::default(),
            task_id: new_task_id(),
            status: TaskStatus::Pending,
            phase_text: String::new(),
            total_bytes: 0,
            copied_bytes: 0,
            total_files: 0,
            copied_files: 0,
            already_bytes: 0,
            speed_bps: 0.0,
            avg_speed_bps: 0.0,
            eta_seconds: None,
            current_file: String::new(),
            returncode: None,
            rc_text: String::new(),
            log_path: String::new(),
            error: String::new(),
            verify_checked: 0,
            verify_passed: 0,
            verify_failed: Vec::new(),
            skipped_count: 0,
            started_at: 0.0,
            finished_at: 0.0,
            elapsed: 0.0,
        }
    }
}

// ---------------------------------------------------------------------- 事件

/// 引擎 → UI 的统一事件。对应 Python 版 `core/models.py` 的 `TaskEvent`
/// （kind + payload dict），Rust 版用带标签的 serde enum 表达。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskEvent {
    /// 某个任务状态/进度发生变化，task 为 CopyTask 副本
    Updated {
        task: CopyTask,
    },
    /// 一行日志
    Log {
        task_id: String,
        line: String,
    },
    /// 任务被删除
    Removed {
        task_id: String,
    },
    /// 引擎调度运行状态变化（开始 / 结束）。
    ///
    /// UI 的「开始 / 停止」按钮必须据此判断，**不要**用「是否存在非终态任务」：
    /// 待执行(PENDING)的任务并不代表引擎在跑，用非终态判断会在刚添加完任务时
    /// 就把「开始」按钮禁用掉（对齐原版 `ui/app.py::_sync_buttons` 的修复）。
    Running {
        running: bool,
    },
    /// 本轮所有任务执行完毕
    AllDone,
    /// 引擎级错误
    Error {
        message: String,
    },
}

// ---------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_clamps_all_numeric_fields() {
        let mut o = CopyOptions::default();
        o.threads = 999;
        o.retries = -3;
        o.wait_seconds = 999;
        let n = o.normalized();
        assert_eq!(n.threads, 128);
        assert_eq!(n.retries, 0);
        assert_eq!(n.wait_seconds, 120);
    }

    #[test]
    fn normalized_zero_threads_falls_back_to_default() {
        let mut o = CopyOptions::default();
        o.threads = 0;
        assert_eq!(o.normalized().threads, 16);
    }

    #[test]
    fn summary_text_contains_mode_and_verify() {
        let mut o = CopyOptions::default();
        o.mirror = true;
        o.dry_run = true;
        let s = o.normalized().summary_text();
        assert!(s.contains("镜像 /MIR"));
        assert!(s.contains("试运行 /L"));
        assert!(s.contains("/MT:16"));
        assert!(s.contains("大小+时间戳"));
    }

    #[test]
    fn percent_handles_unknown_total() {
        let mut t = CopyTask::new("C:\\a", "D:\\b");
        assert_eq!(t.percent(), 0.0);
        t.status = TaskStatus::Done;
        assert_eq!(t.percent(), 100.0);
        t.error = "boom".into();
        assert_eq!(t.percent(), 0.0);
    }

    #[test]
    fn percent_clamps_to_100() {
        let mut t = CopyTask::new("C:\\a", "D:\\b");
        t.total_bytes = 100;
        t.copied_bytes = 200;
        assert_eq!(t.percent(), 100.0);
    }

    #[test]
    fn display_name_falls_back_to_source_tail() {
        let mut t = CopyTask::new("C:\\data\\photos\\", "D:\\backup");
        t.name = "相册备份".into();
        assert_eq!(t.display_name(), "相册备份");
        t.name.clear();
        assert_eq!(t.display_name(), "photos");
    }

    #[test]
    fn task_id_is_8_hex_chars() {
        assert_eq!(CopyTask::new("a", "b").task_id.len(), 8);
        assert!(CopyTask::new("a", "b")
            .task_id
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn touch_elapsed_uses_finished_at_when_present() {
        let mut t = CopyTask::new("a", "b");
        t.started_at = 100.0;
        t.finished_at = 105.5;
        t.touch_elapsed();
        assert!((t.elapsed - 5.5).abs() < 1e-9);
    }
}
