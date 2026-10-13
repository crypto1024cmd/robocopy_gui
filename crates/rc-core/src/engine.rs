//! 任务队列调度引擎。
//!
//! 对应 Python 版 `core/engine.py`。
//!
//! 职责：
//! - 维护任务列表，按串行或指定并行度调度执行；
//! - 每个任务依次执行「扫描源 → 建目标基线 → 运行 robocopy（同时轮询进度）→ 复制后校验」；
//! - 所有状态变化以 [`TaskEvent`] 投递到线程安全通道，UI 只消费事件、
//!   绝不直接读运行中的对象。
//!
//! 线程模型：调度线程 + 每任务一个 worker 线程 + 每任务一个进度轮询线程。
//! 所有对 `CopyTask` 的写入都发生在 worker/轮询线程中（每个任务独立互斥锁），
//! UI 拿到的是快照副本。禁止任何线程直接调用 UI 框架 —— 事件只经通道投递。

use crate::models::{now_secs, CopyTask, TaskEvent, TaskStatus, VerifyLevel, MAX_LINE_CHARS};
use crate::robocopy;
use crate::scanner::{scan_source, TargetProgressTracker};
use crate::verify;
use crate::winfs::{human_bytes, human_duration, robocopy_path};
use crate::{LOG_EMIT_INTERVAL_MS, LOG_TAIL_LINES};
use regex::Regex;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 速度统计的滑动窗口长度（秒）
pub const SPEED_WINDOW_SECS: f64 = 8.0;

const LOG_EMIT_INTERVAL_SECS: f64 = LOG_EMIT_INTERVAL_MS as f64 / 1000.0;

/// 把任意文本转成安全的日志文件名片段。
pub fn safe_filename(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"[\\/:*?"<>|\r\n\t]+"#).expect("文件名正则不合法"));
    let cleaned: String = re.replace_all(text, "_").to_string();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == '_' || c == ' ');
    let out: String = trimmed.chars().take(40).collect();
    if out.is_empty() {
        "task".to_string()
    } else {
        out
    }
}

#[derive(Default)]
struct EngineState {
    tasks: HashMap<String, Arc<Mutex<CopyTask>>>,
    /// 任务插入顺序（保证调度按加入顺序 FIFO）
    order: Vec<String>,
    procs: HashMap<String, u32>,
    stop_flags: HashMap<String, Arc<AtomicBool>>,
    log_files: HashMap<String, Arc<Mutex<std::io::BufWriter<std::fs::File>>>>,
    last_log_emit: HashMap<String, Instant>,
    parallel: usize,
    running: bool,
}

/// 复制任务调度引擎（core 层，不依赖任何 UI 框架）。
///
/// UI 与引擎的唯一通道是 `mpsc::Sender<TaskEvent>`（对应 Python 版的
/// `queue.Queue`）。Tauri 侧由单一事件泵线程 drain 该通道并 emit 给前端。
pub struct CopyEngine {
    events: Sender<TaskEvent>,
    log_dir: PathBuf,
    /// 进度轮询间隔（秒），可由 UI 运行时调整
    poll_interval: Mutex<f64>,
    stop_all: AtomicBool,
    dispatcher: Mutex<Option<JoinHandle<()>>>,
    state: Mutex<EngineState>,
}

impl CopyEngine {
    pub fn new(events: Sender<TaskEvent>, log_dir: &str, poll_interval: f64) -> Arc<Self> {
        Arc::new(Self {
            events,
            log_dir: PathBuf::from(log_dir),
            poll_interval: Mutex::new(if poll_interval < 0.2 { 0.2 } else { poll_interval }),
            stop_all: AtomicBool::new(false),
            dispatcher: Mutex::new(None),
            state: Mutex::new(EngineState {
                parallel: 1,
                ..Default::default()
            }),
        })
    }

    // ------------------------------------------------------------- 任务管理

    pub fn add_task(&self, task: CopyTask) -> CopyTask {
        let arc = Arc::new(Mutex::new(task.clone()));
        {
            let mut st = self.state.lock().unwrap();
            if !st.order.contains(&task.task_id) {
                st.order.push(task.task_id.clone());
            }
            st.tasks.insert(task.task_id.clone(), arc);
        }
        let _ = self.events.send(TaskEvent::Updated { task: task.clone() });
        task
    }

    /// 编辑任务（仅允许编辑未在运行中的任务，对齐原版 _edit_task）。
    pub fn update_task(&self, task: CopyTask) -> bool {
        {
            let st = self.state.lock().unwrap();
            let Some(arc) = st.tasks.get(&task.task_id) else {
                return false;
            };
            let mut t = arc.lock().unwrap();
            if !t.status.is_final() && st.running {
                return false;
            }
            *t = task.clone();
        }
        let _ = self.events.send(TaskEvent::Updated { task });
        true
    }

    /// 删除任务；运行中的任务不允许删除。
    pub fn remove_task(&self, task_id: &str) -> bool {
        {
            let mut st = self.state.lock().unwrap();
            let removable = st
                .tasks
                .get(task_id)
                .map(|t| t.lock().unwrap().status.is_final())
                .unwrap_or(false);
            if !removable {
                return false;
            }
            st.tasks.remove(task_id);
            st.order.retain(|id| id != task_id);
        }
        let _ = self.events.send(TaskEvent::Removed {
            task_id: task_id.to_string(),
        });
        true
    }

    pub fn clear_finished(&self) -> usize {
        let victims: Vec<String> = {
            let mut st = self.state.lock().unwrap();
            let victims: Vec<String> = st
                .order
                .iter()
                .filter(|id| {
                    st.tasks
                        .get(*id)
                        .map(|t| t.lock().unwrap().status.is_final())
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            for id in &victims {
                st.tasks.remove(id);
            }
            st.order.retain(|id| !victims.contains(id));
            victims
        };
        let count = victims.len();
        for id in victims {
            let _ = self.events.send(TaskEvent::Removed { task_id: id });
        }
        count
    }

    /// 把终态任务重置为待执行，供「重新运行」使用。
    pub fn reset_task(&self, task_id: &str) -> bool {
        let snap = {
            let st = self.state.lock().unwrap();
            let Some(task) = st.tasks.get(task_id) else {
                return false;
            };
            let mut t = task.lock().unwrap();
            if !t.status.is_final() || st.running {
                return false;
            }
            t.status = TaskStatus::Pending;
            t.error.clear();
            t.phase_text.clear();
            t.copied_bytes = 0;
            t.already_bytes = 0;
            t.total_bytes = 0;
            t.total_files = 0;
            t.copied_files = 0;
            t.speed_bps = 0.0;
            t.avg_speed_bps = 0.0;
            t.eta_seconds = None;
            t.returncode = None;
            t.rc_text.clear();
            t.verify_checked = 0;
            t.verify_passed = 0;
            t.verify_failed.clear();
            t.elapsed = 0.0;
            t.started_at = 0.0;
            t.finished_at = 0.0;
            t.current_file.clear();
            t.clone()
        };
        let _ = self.events.send(TaskEvent::Updated { task: snap });
        true
    }

    pub fn tasks(&self) -> Vec<CopyTask> {
        let st = self.state.lock().unwrap();
        st.order
            .iter()
            .filter_map(|id| st.tasks.get(id))
            .map(|t| t.lock().unwrap().clone())
            .collect()
    }

    pub fn get(&self, task_id: &str) -> Option<CopyTask> {
        let st = self.state.lock().unwrap();
        st.tasks.get(task_id).map(|t| t.lock().unwrap().clone())
    }

    pub fn is_running(&self) -> bool {
        self.state.lock().unwrap().running
    }

    pub fn running_count(&self) -> usize {
        let st = self.state.lock().unwrap();
        st.order
            .iter()
            .filter(|id| {
                st.tasks
                    .get(*id)
                    .map(|t| !t.lock().unwrap().status.is_final())
                    .unwrap_or(false)
            })
            .count()
    }

    pub fn preview_command(&self, task: &CopyTask) -> String {
        robocopy::preview_command(&task.source, &task.dest, &task.options)
    }

    /// 运行时调整进度轮询间隔（秒）。
    pub fn set_poll_interval(&self, seconds: f64) {
        *self.poll_interval.lock().unwrap() = if seconds < 0.2 { 0.2 } else { seconds };
    }

    // ------------------------------------------------------------- 调度

    /// 开始执行所有待执行任务。
    pub fn start(self: &Arc<Self>, parallel: usize) -> bool {
        {
            let mut st = self.state.lock().unwrap();
            if st.running {
                return false;
            }
            if robocopy_path().is_none() {
                let _ = self.events.send(TaskEvent::Error {
                    message: "未找到 robocopy.exe：本工具依赖 Windows 系统自带的 robocopy".to_string(),
                });
                return false;
            }
            let pending: Vec<String> = st
                .order
                .iter()
                .filter(|id| {
                    st.tasks
                        .get(*id)
                        .map(|t| !t.lock().unwrap().status.is_final())
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if pending.is_empty() {
                let _ = self.events.send(TaskEvent::Error {
                    message: "没有待执行的任务".to_string(),
                });
                return false;
            }
            st.parallel = if parallel == 0 { 1 } else { parallel.clamp(1, 8) };
            self.stop_all.store(false, Ordering::Relaxed);
            st.running = true;
            for id in pending {
                if let Some(t) = st.tasks.get(&id) {
                    let mut task = t.lock().unwrap();
                    task.status = TaskStatus::Pending;
                    task.error.clear();
                }
            }
        }
        // 明确的运行状态事件：UI 按钮据此驱动（PENDING 不代表在跑）
        let _ = self.events.send(TaskEvent::Running { running: true });
        let this = Arc::clone(self);
        let handle = std::thread::Builder::new()
            .name("engine-dispatcher".to_string())
            .spawn(move || this.dispatch_loop());
        *self.dispatcher.lock().unwrap() = handle.ok();
        true
    }

    pub fn stop_task(&self, task_id: &str) {
        let (task_arc, flag, pid) = {
            let st = self.state.lock().unwrap();
            let pending = st
                .tasks
                .get(task_id)
                .map(|t| t.lock().unwrap().status == TaskStatus::Pending)
                .unwrap_or(false);
            if pending {
                (st.tasks.get(task_id).cloned(), None, None)
            } else {
                (None, st.stop_flags.get(task_id).cloned(), st.procs.get(task_id).copied())
            }
        };
        if let Some(task) = task_arc {
            self.edit(&task, |t| {
                t.status = TaskStatus::Stopped;
                t.error = "已被手动取消".to_string();
            });
            self.emit(&task);
            return;
        }
        if let Some(f) = flag {
            f.store(true, Ordering::Relaxed);
        }
        if let Some(pid) = pid {
            robocopy::terminate_pid(pid);
        }
    }

    pub fn stop_all(&self) {
        self.stop_all.store(true, Ordering::Relaxed);
        let (ids, pids, idle) = {
            let st = self.state.lock().unwrap();
            let ids: Vec<String> = st.stop_flags.keys().cloned().collect();
            let pids: Vec<u32> = st.procs.values().copied().collect();
            let idle: Vec<Arc<Mutex<CopyTask>>> = st
                .order
                .iter()
                .filter_map(|id| st.tasks.get(id))
                .filter(|t| t.lock().unwrap().status == TaskStatus::Pending)
                .cloned()
                .collect();
            (ids, pids, idle)
        };
        for t in &idle {
            self.edit(t, |t| {
                t.status = TaskStatus::Stopped;
                t.error = "已被手动取消".to_string();
            });
            self.emit(t);
        }
        for id in ids {
            self.stop_task(&id);
        }
        for pid in pids {
            robocopy::terminate_pid(pid);
        }
    }

    /// 关闭引擎：停止全部任务并等待调度线程退出（排干前的优雅收尾）。
    pub fn shutdown(&self) {
        self.stop_all();
        let handle = self.dispatcher.lock().unwrap().take();
        if let Some(h) = handle {
            let _ = h.join();
        }
    }

    // ------------------------------------------------------------- 内部：调度

    fn dispatch_loop(self: Arc<Self>) {
        let mut threads: HashMap<String, JoinHandle<()>> = HashMap::new();
        while !self.stop_all.load(Ordering::Relaxed) {
            let (parallel, mut candidates): (usize, Vec<String>) = {
                let st = self.state.lock().unwrap();
                threads.retain(|_, th| !th.is_finished());
                let candidates = st
                    .order
                    .iter()
                    .filter(|id| {
                        !threads.contains_key(*id)
                            && st.tasks
                                .get(*id)
                                .map(|t| t.lock().unwrap().status == TaskStatus::Pending)
                                .unwrap_or(false)
                    })
                    .cloned()
                    .collect();
                (st.parallel, candidates)
            };
            while !candidates.is_empty()
                && threads.len() < parallel
                && !self.stop_all.load(Ordering::Relaxed)
            {
                let id = candidates.remove(0);
                let this = Arc::clone(&self);
                let tid = id.clone();
                if let Ok(h) = std::thread::Builder::new()
                    .name(format!("task-{}", id))
                    .spawn(move || this.run_task(tid))
                {
                    threads.insert(id, h);
                }
            }
            if candidates.is_empty() && threads.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(120));
        }

        for (_, th) in threads {
            let _ = th.join();
        }
        {
            let mut st = self.state.lock().unwrap();
            st.running = false;
        }
        let _ = self.events.send(TaskEvent::Running { running: false });
        let _ = self.events.send(TaskEvent::AllDone);
    }

    fn run_task(self: &Arc<Self>, task_id: String) {
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut st = self.state.lock().unwrap();
            st.stop_flags.insert(task_id.clone(), stop.clone());
        }
        let task = {
            let st = self.state.lock().unwrap();
            st.tasks.get(&task_id).cloned()
        };
        let Some(task) = task else { return };

        // 任何异常都不能让调度线程挂掉
        if let Err(msg) = self.execute(&task, &task_id, &stop) {
            self.edit(&task, |t| {
                t.status = TaskStatus::Failed;
                t.error = msg.clone();
            });
            self.log(&task_id, &format!("任务异常终止：{}", msg), true, false);
        }

        // ---- finally：收尾 ----
        {
            let mut st = self.state.lock().unwrap();
            st.stop_flags.remove(&task_id);
            st.procs.remove(&task_id);
        }
        self.edit(&task, |t| {
            t.finished_at = now_secs();
            t.touch_elapsed();
            if !t.status.is_final() {
                t.status = TaskStatus::Failed;
                if t.error.is_empty() {
                    t.error = "任务未正常结束".to_string();
                }
            }
            t.speed_bps = 0.0;
            t.eta_seconds = None;
        });
        let (name, status_label, elapsed) = {
            let t = task.lock().unwrap();
            (t.display_name().to_string(), t.status.label(), t.elapsed)
        };
        self.log(
            &task_id,
            &format!(
                "任务「{}」结束：{}，耗时 {}",
                name,
                status_label,
                human_duration(Some(elapsed))
            ),
            true,
            false,
        );
        let handle = {
            let mut st = self.state.lock().unwrap();
            st.log_files.remove(&task_id)
        };
        if let Some(handle) = handle {
            if let Ok(mut w) = handle.lock() {
                let _ = w.flush();
            }
        }
        self.emit(&task);
    }

    // ------------------------------------------------------------- 内部：单任务

    fn execute(
        self: &Arc<Self>,
        task: &Arc<Mutex<CopyTask>>,
        task_id: &str,
        stop: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        self.edit(task, |t| {
            t.started_at = now_secs();
            t.finished_at = 0.0;
            t.error.clear();
            t.verify_failed.clear();
            t.verify_checked = 0;
            t.verify_passed = 0;
            t.copied_bytes = 0;
            t.already_bytes = 0;
            t.returncode = None;
            t.rc_text.clear();
            t.current_file.clear();
        });

        self.set_status(task, TaskStatus::Scanning);
        let (display_name, route, summary, source) = {
            let t = task.lock().unwrap();
            (
                t.display_name().to_string(),
                t.route_text(),
                t.options.summary_text(),
                t.source.clone(),
            )
        };
        self.log(task_id, &format!("任务「{}」开始：{}", display_name, route), true, false);
        self.log(task_id, &format!("复制参数：{}", summary), true, false);

        if !std::path::Path::new(&source).is_dir() {
            return Err(format!("源目录不存在或不是文件夹：{}", source));
        }

        // ---- 1. 扫描源目录，建立清单 ----
        let mut on_scan = |count: usize, total_bytes: u64| {
            self.edit(task, |t| {
                t.total_files = count as u64;
                t.total_bytes = total_bytes;
                t.phase_text =
                    format!("已发现 {} 个文件 / {}", count, human_bytes(total_bytes as f64));
            });
            self.emit(task);
        };
        let excludes = vec![self.log_dir.to_string_lossy().into_owned()];
        let scan = scan_source(
            &source,
            Some(stop),
            &excludes,
            Some(&mut on_scan),
            0.3,
        );
        self.edit(task, |t| {
            t.total_files = scan.file_count as u64;
            t.total_bytes = scan.total_bytes;
            t.skipped_count = scan.skipped_count as u64;
        });
        if scan.cancelled {
            self.finish_stopped(task, task_id);
            return Ok(());
        }
        for note in scan.skipped.iter().take(10) {
            self.log(task_id, &format!("扫描时跳过：{}", note), true, false);
        }
        if scan.skipped_count > 10 {
            self.log(
                task_id,
                &format!("扫描时共跳过 {} 项（权限或路径异常）", scan.skipped_count),
                true,
                false,
            );
        }
        self.log(
            task_id,
            &format!(
                "源目录共 {} 个文件 / {} 个目录，合计 {}",
                scan.file_count,
                scan.dirs.len(),
                human_bytes(scan.total_bytes as f64)
            ),
            true,
            false,
        );
        self.emit(task);

        // ---- 2. 目标目录基线：算出已经是最新的部分，并精简后续轮询范围 ----
        let dest = task.lock().unwrap().dest.clone();
        if !std::path::Path::new(&dest).is_dir() {
            if let Err(exc) = std::fs::create_dir_all(&dest) {
                return Err(format!("无法创建目标目录 {}：{}", dest, exc));
            }
        }
        self.edit(task, |t| t.phase_text = "正在比对目标目录…".to_string());
        self.emit(task);

        let tracker = Arc::new(TargetProgressTracker::new(&scan, &dest));
        tracker.build_baseline(None);
        self.edit(task, |t| {
            t.copied_bytes = tracker.done_bytes();
            t.copied_files = tracker.done_files();
            t.already_bytes = tracker.uptodate_bytes();
        });
        self.log(
            task_id,
            &format!(
                "目标端原有 {} 个同名同大小文件（{}），其中 {} 个时间戳一致，robocopy 会跳过",
                tracker.done_files(),
                human_bytes(tracker.done_bytes() as f64),
                tracker.uptodate_files()
            ),
            true,
            false,
        );
        self.emit(task);

        // ---- 3. 准备日志文件 ----
        let log_path = self.new_log_path(task);
        let _ = std::fs::create_dir_all(&self.log_dir);
        match std::fs::File::create(&log_path) {
            Ok(f) => {
                let mut st = self.state.lock().unwrap();
                st.log_files.insert(
                    task_id.to_string(),
                    Arc::new(Mutex::new(std::io::BufWriter::new(f))),
                );
                drop(st);
                self.edit(task, |t| {
                    t.log_path = log_path.to_string_lossy().into_owned();
                });
            }
            Err(_) => {
                self.edit(task, |t| t.log_path.clear());
            }
        }

        // ---- 4. 运行 robocopy，同时轮询目标目录计算进度 ----
        let snap = task.lock().unwrap();
        let cmd = robocopy::build_command(&snap.source, &snap.dest, &snap.options);
        let dry_run = snap.options.dry_run;
        drop(snap);
        self.log(
            task_id,
            &format!("命令行：{}", robocopy::list_to_command_line(&cmd)),
            true,
            false,
        );

        self.set_status(task, TaskStatus::Running);
        self.edit(task, |t| {
            t.phase_text = if dry_run {
                "试运行：仅列出将要复制的文件，不写入磁盘".to_string()
            } else {
                "多线程复制中".to_string()
            };
        });
        let run_start = Instant::now();
        let start_bytes = tracker.done_bytes();
        let mut raw_tail: VecDeque<String> = VecDeque::with_capacity(LOG_TAIL_LINES + 8);

        let monitor_stop = Arc::new(AtomicBool::new(false));
        let mut monitor: Option<JoinHandle<()>> = None;
        if !dry_run {
            let this = Arc::clone(self);
            let task_arc = Arc::clone(task);
            let tracker_arc = Arc::clone(&tracker);
            let stop_flag = Arc::clone(&monitor_stop);
            let tid = task_id.to_string();
            monitor = std::thread::Builder::new()
                .name(format!("monitor-{}", task_id))
                .spawn(move || {
                    this.monitor_loop(task_arc, tid, tracker_arc, stop_flag, run_start, start_bytes)
                })
                .ok();
        }

        let mut on_line = |line: &str| {
            raw_tail.push_back(line.to_string());
            if raw_tail.len() > LOG_TAIL_LINES {
                raw_tail.pop_front();
            }
            match robocopy::parse_file_line(line) {
                None => self.log(task_id, line, false, true),
                Some(parsed) => {
                    tracker.hint(&parsed.path);
                    if parsed.is_file() {
                        let base = parsed
                            .path
                            .rsplit(['\\', '/'])
                            .next()
                            .unwrap_or("")
                            .to_string();
                        self.edit(task, |t| t.current_file = base);
                    }
                    self.log(task_id, line, false, false);
                }
            }
        };
        let register = |pid: u32| {
            let mut st = self.state.lock().unwrap();
            st.procs.insert(task_id.to_string(), pid);
        };
        // 日志由本引擎统一写入，避免两处抢同一个文件
        let run_result = robocopy::run_robocopy(
            &cmd,
            None,
            Some(&mut on_line),
            Some(stop),
            Some(&register),
            None,
        );
        drop(on_line);
        monitor_stop.store(true, Ordering::Relaxed);
        if let Some(h) = monitor {
            let _ = h.join();
        }
        {
            let mut st = self.state.lock().unwrap();
            st.procs.remove(task_id);
        }
        let returncode = match run_result {
            Ok(rc) => rc,
            Err(exc) => {
                return Err(format!("无法启动 robocopy：{}", exc));
            }
        };

        tracker.final_poll();
        self.edit(task, |t| {
            t.copied_bytes = tracker.done_bytes();
            t.copied_files = tracker.done_files();
            t.returncode = Some(returncode);
            t.rc_text = robocopy::exit_code_text(returncode);
            t.speed_bps = 0.0;
            t.eta_seconds = None;
            t.touch_elapsed();
        });
        self.emit(task);

        self.log(task_id, &format!("进度统计：{}", tracker.diagnostics()), true, false);

        if stop.load(Ordering::Relaxed) || returncode < 0 {
            self.finish_stopped(task, task_id);
            return Ok(());
        }

        // ---- 解析 robocopy 末尾统计表（内存滑窗，不重读大日志） ----
        let summary = robocopy::parse_summary_text(
            &raw_tail.iter().cloned().collect::<Vec<_>>().join("\n"),
        );
        if summary.files_copied > 0 || summary.files_total > 0 {
            self.log(
                task_id,
                &format!(
                    "robocopy 统计：文件 共 {} / 复制 {} / 跳过 {} / 失败 {}，字节 {}",
                    summary.files_total,
                    summary.files_copied,
                    summary.files_skipped,
                    summary.files_failed,
                    human_bytes(summary.bytes_copied as f64)
                ),
                true,
                false,
            );
        }

        if !robocopy::is_success(returncode) {
            self.edit(task, |t| {
                t.status = TaskStatus::Failed;
                t.error = format!("robocopy 返回失败：{}", t.rc_text);
            });
            let err = task.lock().unwrap().error.clone();
            self.log(task_id, &err, true, false);
            return Ok(());
        }

        self.log(
            task_id,
            &format!("robocopy 正常结束：{}", task.lock().unwrap().rc_text),
            true,
            false,
        );
        if dry_run {
            self.edit(task, |t| {
                t.status = TaskStatus::Done;
                t.phase_text = "试运行完成（未写入任何文件）".to_string();
            });
            self.log(
                task_id,
                "试运行结束，以上为将要复制的范围；确认无误后可关闭试运行正式复制",
                true,
                false,
            );
            return Ok(());
        }

        // ---- 5. 复制后校验 ----
        let level = task.lock().unwrap().options.verify_level;
        if level == VerifyLevel::None {
            self.edit(task, |t| {
                t.status = TaskStatus::Done;
                t.phase_text = "复制完成（未校验）".to_string();
            });
            self.log(task_id, "任务完成，未执行校验", true, false);
            return Ok(());
        }

        self.set_status(task, TaskStatus::Verifying);
        self.edit(task, |t| {
            t.phase_text = format!("正在校验（{}）…", level.label());
        });

        let mut on_verify = |checked: usize, total: usize| {
            self.edit(task, |t| {
                t.verify_checked = checked as u64;
                t.phase_text = format!("正在校验 {}/{}", checked, total);
            });
            self.emit(task);
        };
        let algo = task.lock().unwrap().options.hash_algo;
        let result = verify::verify_task(
            &scan,
            &dest,
            level,
            algo,
            Some(&mut on_verify),
            Some(Arc::clone(stop)),
            verify::DEFAULT_WORKERS,
            crate::scanner::MTIME_TOLERANCE_NS,
        );
        self.edit(task, |t| {
            t.verify_checked = result.checked;
            t.verify_passed = result.passed;
            t.verify_failed = result.failed.clone();
            t.touch_elapsed();
        });

        if result.cancelled {
            self.finish_stopped(task, task_id);
            return Ok(());
        }
        if result.ok() {
            self.edit(task, |t| {
                t.status = TaskStatus::Done;
                t.error.clear();
                t.phase_text = format!(
                    "完成并通过校验（{} 个文件，耗时 {}）",
                    result.checked,
                    human_duration(Some(result.elapsed))
                );
            });
            self.log(
                task_id,
                &format!("校验通过：{}/{} 个文件一致", result.passed, result.checked),
                true,
                false,
            );
        } else {
            self.edit(task, |t| {
                t.status = TaskStatus::Failed;
                t.error = format!(
                    "校验未通过：{}/{} 个文件不一致",
                    result.failed_count, result.checked
                );
                t.phase_text = t.error.clone();
            });
            let err = task.lock().unwrap().error.clone();
            self.log(task_id, &err, true, false);
            for diff in result.failed.iter().take(10) {
                self.log(
                    task_id,
                    &format!("  校验失败：{} —— {}", diff.path, diff.reason),
                    true,
                    false,
                );
            }
            if result.failed_count > 10 {
                self.log(
                    task_id,
                    &format!("  另有 {} 个文件校验失败", result.failed_count - 10),
                    true,
                    false,
                );
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------- 内部：轮询

    fn monitor_loop(
        &self,
        task: Arc<Mutex<CopyTask>>,
        _task_id: String,
        tracker: Arc<TargetProgressTracker>,
        monitor_stop: Arc<AtomicBool>,
        run_start: Instant,
        start_bytes: u64,
    ) {
        let mut samples: VecDeque<(Instant, u64)> = VecDeque::new();
        let interval = Duration::from_secs_f64(*self.poll_interval.lock().unwrap());
        let mut next_poll = Instant::now() + interval;
        loop {
            if monitor_stop.load(Ordering::Relaxed) {
                break;
            }
            if Instant::now() < next_poll {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            next_poll = Instant::now() + interval;

            let copied = tracker.poll();
            let now = Instant::now();
            samples.push_back((now, copied));
            while samples.len() > 64 {
                samples.pop_front();
            }
            let cutoff = now - Duration::from_secs_f64(SPEED_WINDOW_SECS);
            while samples.len() > 2 && samples.front().unwrap().0 < cutoff {
                samples.pop_front();
            }
            let mut speed = 0.0f64;
            if samples.len() >= 2 {
                let (t0, b0) = samples.front().unwrap();
                let (t1, b1) = samples.back().unwrap();
                let span = t1.duration_since(*t0).as_secs_f64();
                if span > 0.0 {
                    speed = (b1.saturating_sub(*b0) as f64 / span).max(0.0);
                }
            }
            let elapsed = run_start.elapsed().as_secs_f64();

            {
                let mut t = task.lock().unwrap();
                t.speed_bps = speed;
                if elapsed > 0.5 {
                    t.avg_speed_bps = (copied.saturating_sub(start_bytes) as f64 / elapsed).max(0.0);
                }
                t.copied_bytes = copied;
                t.copied_files = tracker.done_files();
                let remaining = t.total_bytes.saturating_sub(copied);
                let spd = if t.avg_speed_bps != 0.0 {
                    t.avg_speed_bps
                } else {
                    t.speed_bps
                };
                t.eta_seconds = if remaining == 0 {
                    Some(0.0)
                } else if spd > 0.0 {
                    Some(remaining as f64 / spd)
                } else {
                    None
                };
                t.touch_elapsed();
                let snap = t.clone();
                drop(t);
                let _ = self.events.send(TaskEvent::Updated { task: snap });
            }
        }
    }

    // ------------------------------------------------------------- 内部：工具

    fn edit(&self, task: &Mutex<CopyTask>, f: impl FnOnce(&mut CopyTask)) {
        let mut t = task.lock().unwrap();
        f(&mut t);
    }

    fn emit(&self, task: &Mutex<CopyTask>) {
        let snap = task.lock().unwrap().clone();
        let _ = self.events.send(TaskEvent::Updated { task: snap });
    }

    fn set_status(&self, task: &Mutex<CopyTask>, status: TaskStatus) {
        self.edit(task, |t| t.status = status);
        self.emit(task);
    }

    fn finish_stopped(&self, task: &Mutex<CopyTask>, task_id: &str) {
        self.edit(task, |t| {
            t.status = TaskStatus::Stopped;
            t.speed_bps = 0.0;
            t.eta_seconds = None;
            t.error = "任务已手动停止，目标目录中可能残留不完整的文件".to_string();
            t.phase_text = "已停止".to_string();
            t.touch_elapsed();
        });
        let msg = "任务已手动停止，目标目录中可能残留不完整的文件".to_string();
        self.log(task_id, &msg, true, false);
        self.emit(task);
    }

    fn new_log_path(&self, task: &Mutex<CopyTask>) -> PathBuf {
        let (display_name, task_id) = {
            let t = task.lock().unwrap();
            (t.display_name().to_string(), t.task_id.clone())
        };
        let (y, mo, d, h, mi, s) = local_time_parts();
        self.log_dir.join(format!(
            "{:04}{:02}{:02}_{:02}{:02}{:02}_{}_{}.log",
            y,
            mo,
            d,
            h,
            mi,
            s,
            safe_filename(&display_name),
            task_id
        ))
    }

    /// 写日志：完整内容落盘，节流后推送给 UI。
    fn log(&self, task_id: &str, line: &str, own: bool, force: bool) {
        let mut text = line.trim_end_matches(['\r', '\n']).to_string();
        if text.trim().is_empty() {
            return;
        }
        if text.chars().count() > MAX_LINE_CHARS {
            text = format!("{} …", text.chars().take(MAX_LINE_CHARS).collect::<String>());
        }
        if own {
            text = format!("» {}", text);
        }
        let now = Instant::now();
        let mut should_emit = force;
        {
            let mut st = self.state.lock().unwrap();
            if let Some(w) = st.log_files.get(task_id) {
                if let Ok(mut w) = w.lock() {
                    let _ = writeln!(w, "{}", text);
                }
            }
            if !force {
                let throttle_ok = st
                    .last_log_emit
                    .get(task_id)
                    .map(|l| now.duration_since(*l).as_secs_f64() >= LOG_EMIT_INTERVAL_SECS)
                    .unwrap_or(true);
                if throttle_ok {
                    st.last_log_emit.insert(task_id.to_string(), now);
                    should_emit = true;
                }
            }
        }
        if should_emit {
            let _ = self.events.send(TaskEvent::Log {
                task_id: task_id.to_string(),
                line: text,
            });
        }
    }
}

/// 本地时间（年月日时分秒），日志文件名时间戳使用。
#[cfg(windows)]
fn local_time_parts() -> (i32, u32, u32, u32, u32, u32) {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    (
        st.wYear as i32,
        st.wMonth as u32,
        st.wDay as u32,
        st.wHour as u32,
        st.wMinute as u32,
        st.wSecond as u32,
    )
}

#[cfg(not(windows))]
fn local_time_parts() -> (i32, u32, u32, u32, u32, u32) {
    // 非 Windows 平台退化为 UTC，仅用于日志命名
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    let rem = secs % 86_400;
    (y as i32, m, d, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
}

#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CopyOptions;
    use crate::models::VerifyLevel;
    use std::sync::mpsc;

    #[test]
    fn safe_filename_replaces_bad_chars() {
        assert_eq!(safe_filename("my<task>|2024"), "my_task_2024");
        assert_eq!(safe_filename("  a.b.c  "), "a.b.c");
        assert_eq!(safe_filename(""), "task");
        let long = "x".repeat(100);
        assert_eq!(safe_filename(&long).len(), 40);
        let path_like = "C:\\data\\photos";
        assert_eq!(safe_filename(path_like), "C_data_photos");
    }

    #[test]
    fn add_and_remove_task_lifecycle() {
        let (tx, _rx) = mpsc::channel();
        let engine = CopyEngine::new(tx, "logs", 1.0);
        let task = engine.add_task(CopyTask::new("C:\\a", "D:\\b"));
        assert_eq!(engine.tasks().len(), 1);
        assert_eq!(engine.running_count(), 1);
        // PENDING（非终态）不允许删除
        assert!(!engine.remove_task(&task.task_id));
        // 置为终态后可删除
        {
            let st = engine.state.lock().unwrap();
            let t = st.tasks.get(&task.task_id).unwrap();
            t.lock().unwrap().status = TaskStatus::Done;
        }
        assert!(engine.remove_task(&task.task_id));
        assert_eq!(engine.tasks().len(), 0);
    }

    #[test]
    fn reset_task_restores_pending() {
        let (tx, _rx) = mpsc::channel();
        let engine = CopyEngine::new(tx, "logs", 1.0);
        let task_id = engine.add_task(CopyTask::new("C:\\a", "D:\\b")).task_id;
        {
            let st = engine.state.lock().unwrap();
            let t = st.tasks.get(&task_id).unwrap();
            t.lock().unwrap().status = TaskStatus::Done;
        }
        assert!(engine.reset_task(&task_id));
        assert_eq!(engine.get(&task_id).unwrap().status, TaskStatus::Pending);
    }

    #[test]
    #[cfg(windows)]
    fn engine_end_to_end_copy_with_quick_verify() {
        let base = std::env::temp_dir();
        let tag = format!("rc_eng_{}", std::process::id());
        let src = base.join(format!("{}_src", tag));
        let dst = base.join(format!("{}_dst", tag));
        let logs = base.join(format!("{}_logs", tag));
        for p in [&src, &dst, &logs] {
            let _ = std::fs::remove_dir_all(p);
        }
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "alpha").unwrap();
        std::fs::write(src.join("sub\\b.txt"), "bravo").unwrap();

        let (tx, rx) = mpsc::channel::<TaskEvent>();
        let engine = CopyEngine::new(tx, &logs.to_string_lossy(), 1.0);

        let mut task = CopyTask::new(
            src.to_string_lossy().into_owned(),
            dst.to_string_lossy().into_owned(),
        );
        task.name = "端到端测试".to_string();
        task.options = CopyOptions {
            verify_level: VerifyLevel::Quick,
            ..Default::default()
        };
        let task_id = engine.add_task(task).task_id;

        assert!(engine.start(1));
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut saw_all_done = false;
        let mut saw_log = false;
        let mut saw_running_start = false;
        let mut saw_running_end = false;
        loop {
            if Instant::now() > deadline {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(TaskEvent::AllDone) => {
                    saw_all_done = true;
                    break;
                }
                Ok(TaskEvent::Running { running }) => {
                    if running {
                        saw_running_start = true;
                    } else {
                        saw_running_end = true;
                    }
                }
                Ok(TaskEvent::Log { .. }) => {
                    saw_log = true;
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        engine.shutdown();
        assert!(saw_all_done, "应在超时前收到 all_done 事件");
        assert!(saw_log, "应有日志事件推送到 UI 通道");
        // 运行状态事件必须成对出现：UI 按钮据此启用「开始/停止」，不能靠任务状态推断
        assert!(saw_running_start, "start 应推送 running=true 事件");
        assert!(saw_running_end, "调度结束应推送 running=false 事件");

        let tasks = engine.tasks();
        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];
        assert_eq!(t.status, TaskStatus::Done, "任务应成功完成，错误：{}", t.error);
        assert_eq!(t.returncode, Some(1), "两个新文件 → robocopy 返回 1");
        assert_eq!(t.verify_checked, 2);
        assert_eq!(t.verify_passed, 2);
        assert_eq!(t.verify_failed.len(), 0);
        assert!(!t.log_path.is_empty(), "日志应完整落盘");
        let log_content = std::fs::read_to_string(&t.log_path).unwrap_or_default();
        // 注意：「任务开始」发生在日志文件创建之前（与原版一致），只推 UI 不落盘
        assert!(log_content.contains("命令行："), "日志应含命令行，实际：{}", log_content);
        assert!(log_content.contains("robocopy 统计"));
        assert!(log_content.contains("校验通过"));

        // 目标端确实复制成功
        assert_eq!(
            std::fs::read_to_string(dst.join("a.txt")).unwrap(),
            "alpha"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("sub\\b.txt")).unwrap(),
            "bravo"
        );

        // 复跑：reset 后再次执行（目标已最新 → rc=0）
        assert!(engine.reset_task(&task_id));
        assert!(engine.start(1));
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if Instant::now() > deadline {
                break;
            }
            match rx.try_recv() {
                Ok(TaskEvent::AllDone) => break,
                Ok(_) => continue,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        engine.shutdown();
        let t = engine.get(&task_id).unwrap();
        assert_eq!(t.status, TaskStatus::Done);
        assert_eq!(t.returncode, Some(0), "第二次运行目标已最新，robocopy 返回 0");

        // 清理
        for p in [&src, &dst, &logs] {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}
