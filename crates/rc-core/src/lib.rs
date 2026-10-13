//! rc-core：纯逻辑库，对应 Python 版 `core/` 目录。
//!
//! 模块划分（与 Python 版一一对应）：
//! - [`models`]  数据契约：CopyTask / CopyOptions / TaskStatus / TaskEvent
//! - [`winfs`]   盘符枚举、容量、长路径、路径关系、格式化
//! - [`scanner`] 源目录扫描 + 目标端增量轮询进度追踪
//! - [`robocopy`]命令拼装、子进程运行、输出解码、统计表解析
//! - [`verify`]  三级校验（大小+时间戳 / 哈希）
//! - [`engine`]  任务调度、worker 线程、日志节流落盘、事件投递
//!
//! 铁律：本 crate 不得依赖 tauri 或任何 UI 框架（对齐原版「core 不碰 UI」）。
//! 引擎 → UI 的唯一通道是 mpsc 投递的 [`models::TaskEvent`]。

pub mod engine;
pub mod models;
pub mod robocopy;
pub mod scanner;
pub mod verify;
pub mod winfs;

/// UI 日志推送最小间隔（毫秒）——节流护栏，禁止无节流把 robocopy 逐行输出刷上界面。
pub const LOG_EMIT_INTERVAL_MS: u64 = 40;

/// 解析 robocopy 末尾统计表的内存滑窗行数——不要把整份大日志常驻内存。
pub const LOG_TAIL_LINES: usize = 300;

pub use models::MAX_LINE_CHARS;

/// 日志完整落盘目录（相对当前工作目录），对应 Python 版的 logs/。
pub const LOG_DIR: &str = "logs";
