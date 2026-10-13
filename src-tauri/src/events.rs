//! 引擎事件 → Tauri 前端的唯一推送出口。
//!
//! rc-core 引擎把 `TaskEvent` 投递进 mpsc 通道；本模块用单一事件泵线程
//! drain 后 `emit` 给 WebView。任何 worker 线程都不直接触碰 UI。

use rc_core::models::TaskEvent;
use std::sync::mpsc::Receiver;
use tauri::{AppHandle, Emitter};

/// 前端 `listen()` 订阅的事件名。
pub const TASK_EVENT_CHANNEL: &str = "task-event";

pub fn spawn_event_pump(app: AppHandle, rx: Receiver<TaskEvent>) {
    std::thread::Builder::new()
        .name("event-pump".to_string())
        .spawn(move || {
            // 通道断开（引擎销毁）即退出；排干语义由通道顺序保证
            while let Ok(event) = rx.recv() {
                let _ = app.emit(TASK_EVENT_CHANNEL, &event);
            }
        })
        .ok();
}
