//! Tauri invoke command 集：前端 → 引擎的唯一入口。
//!
//! 重 I/O 的 command（源大小估算等）标记为 async，避免阻塞主线程；
//! 轻量 command 保持同步。所有状态都从 `AppState.engine` 走，
//! command 层不持有可变业务状态。

use crate::AppState;
use rc_core::models::CopyTask;
use rc_core::robocopy;
use rc_core::scanner::scan_source;
use rc_core::winfs;
use tauri::State;

#[derive(serde::Serialize)]
pub struct ScanEstimate {
    pub files: usize,
    pub dirs: usize,
    pub total_bytes: u64,
}

#[derive(serde::Serialize)]
pub struct DiskFreeHint {
    pub ok: bool,
    pub text: String,
}

// ---------------------------------------------------------------------- 任务

#[tauri::command]
pub fn list_tasks(state: State<'_, AppState>) -> Vec<CopyTask> {
    state.engine.tasks()
}

#[tauri::command]
pub fn add_task(state: State<'_, AppState>, task: CopyTask) -> CopyTask {
    state.engine.add_task(task)
}

#[tauri::command]
pub fn update_task(state: State<'_, AppState>, task: CopyTask) -> bool {
    state.engine.update_task(task)
}

#[tauri::command]
pub fn remove_task(state: State<'_, AppState>, task_id: String) -> bool {
    state.engine.remove_task(&task_id)
}

#[tauri::command]
pub fn clear_finished(state: State<'_, AppState>) -> usize {
    state.engine.clear_finished()
}

#[tauri::command]
pub fn reset_task(state: State<'_, AppState>, task_id: String) -> bool {
    state.engine.reset_task(&task_id)
}

// ---------------------------------------------------------------------- 调度

#[tauri::command]
pub fn start_all(state: State<'_, AppState>, parallel: usize) -> bool {
    state.engine.start(parallel)
}

#[tauri::command]
pub fn stop_task(state: State<'_, AppState>, task_id: String) {
    state.engine.stop_task(&task_id)
}

#[tauri::command]
pub fn stop_all(state: State<'_, AppState>) {
    state.engine.stop_all()
}

#[tauri::command]
pub fn is_running(state: State<'_, AppState>) -> bool {
    state.engine.is_running()
}

#[tauri::command]
pub fn running_count(state: State<'_, AppState>) -> usize {
    state.engine.running_count()
}

/// 运行时调整引擎进度轮询间隔（右侧栏「进度刷新」下拉）。
#[tauri::command]
pub fn set_poll_interval(state: State<'_, AppState>, seconds: f64) {
    state.engine.set_poll_interval(seconds);
}

/// 系统目录选择对话框（任务编辑器「浏览…」按钮）。
#[tauri::command]
pub async fn pick_folder(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("选择文件夹")
            .blocking_pick_folder()
    })
    .await
    .ok()??;
    Some(picked.to_string())
}

// ---------------------------------------------------------------------- 工具

#[tauri::command]
pub fn preview_command(task: CopyTask) -> String {
    robocopy::preview_command(&task.source, &task.dest, &task.options)
}

#[tauri::command]
pub fn list_drives() -> Vec<winfs::DriveInfo> {
    winfs::list_drives()
}

#[tauri::command]
pub fn check_paths(src: String, dst: String) -> winfs::PathRelation {
    winfs::path_relation(&src, &dst)
}

/// 源目录大小估算（任务编辑器）。重 I/O → async，跑在独立线程池。
#[tauri::command]
pub async fn scan_estimate(src: String) -> Result<ScanEstimate, String> {
    let scan = tauri::async_runtime::spawn_blocking(move || scan_source(&src, None, &[], None, 0.3))
        .await
        .map_err(|e| e.to_string())?;
    Ok(ScanEstimate {
        files: scan.file_count,
        dirs: scan.dirs.len(),
        total_bytes: scan.total_bytes,
    })
}

#[tauri::command]
pub fn disk_free_hint(src_bytes: u64, dest: String) -> DiskFreeHint {
    let (ok, text) = winfs::free_space_hint(src_bytes, &dest);
    DiskFreeHint { ok, text }
}

/// 用资源管理器打开目录（日志目录、任务目标目录等）。
#[tauri::command]
pub fn open_path(path: String) -> bool {
    winfs::open_in_explorer(&path)
}

/// 日志查看器：只读尾部 32KB，避免大日志整读。
#[tauri::command]
pub fn read_log_tail(path: String) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let size = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    let start = size.saturating_sub(32 * 1024);
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 任务完成系统通知（前端在任务达到终态时调用）。
#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}
