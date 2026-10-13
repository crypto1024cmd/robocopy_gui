//! Tauri 应用装配：状态注册、事件泵启动与退出清理。

mod commands;
mod events;

use rc_core::engine::CopyEngine;
use rc_core::models::TaskEvent;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// 全局共享状态：调度引擎 + 事件接收端（由事件泵取走）。
pub struct AppState {
    pub engine: Arc<CopyEngine>,
    pub rx: Mutex<Option<Receiver<TaskEvent>>>,
}

pub fn run() {
    // 引擎 → UI 唯一通道；接收端交给事件泵线程
    let (tx, rx) = std::sync::mpsc::channel::<TaskEvent>();
    // 日志完整落盘到 logs/（相对当前工作目录，与 rc-core::LOG_DIR 保持一致）
    let engine = CopyEngine::new(tx, rc_core::LOG_DIR, 1.0);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            engine,
            rx: Mutex::new(Some(rx)),
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_tasks,
            commands::add_task,
            commands::update_task,
            commands::remove_task,
            commands::clear_finished,
            commands::reset_task,
            commands::start_all,
            commands::stop_task,
            commands::stop_all,
            commands::is_running,
            commands::running_count,
            commands::preview_command,
            commands::pick_folder,
            commands::set_poll_interval,
            commands::list_drives,
            commands::check_paths,
            commands::scan_estimate,
            commands::disk_free_hint,
            commands::open_path,
            commands::read_log_tail,
            commands::notify,
        ])
        .setup(|app| {
            let state = app.state::<AppState>();
            if let Some(rx) = state.rx.lock().unwrap().take() {
                events::spawn_event_pump(app.handle().clone(), rx);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build robocopy-gui");

    app.run(|app_handle, event| {
        // 关闭程序前：终止所有 worker → 排干队列 → 退出，不可暴力跳过清理
        if matches!(event, tauri::RunEvent::Exit) {
            app_handle.state::<AppState>().engine.shutdown();
        }
    });
}
