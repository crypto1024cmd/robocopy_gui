/** invoke / listen 封装：前端与 Tauri 后端的唯一出入口。 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DiskFreeHint,
  DriveInfo,
  PathRelation,
  ScanEstimate,
  TaskEvent,
  TaskSnapshot,
} from "./types";

export const TASK_EVENT_CHANNEL = "task-event";

export const api = {
  listTasks: () => invoke<TaskSnapshot[]>("list_tasks"),
  addTask: (task: TaskSnapshot) => invoke<TaskSnapshot>("add_task", { task }),
  updateTask: (task: TaskSnapshot) => invoke<boolean>("update_task", { task }),
  removeTask: (taskId: string) => invoke<boolean>("remove_task", { taskId }),
  clearFinished: () => invoke<number>("clear_finished"),
  resetTask: (taskId: string) => invoke<boolean>("reset_task", { taskId }),

  startAll: (parallel: number) => invoke<boolean>("start_all", { parallel }),
  stopTask: (taskId: string) => invoke<void>("stop_task", { taskId }),
  stopAll: () => invoke<void>("stop_all"),
  isRunning: () => invoke<boolean>("is_running"),
  runningCount: () => invoke<number>("running_count"),
  setPollInterval: (seconds: number) => invoke<void>("set_poll_interval", { seconds }),

  previewCommand: (task: TaskSnapshot) => invoke<string>("preview_command", { task }),
  listDrives: () => invoke<DriveInfo[]>("list_drives"),
  checkPaths: (src: string, dst: string) => invoke<PathRelation>("check_paths", { src, dst }),
  scanEstimate: (src: string) => invoke<ScanEstimate>("scan_estimate", { src }),
  diskFreeHint: (srcBytes: number, dest: string) =>
    invoke<DiskFreeHint>("disk_free_hint", { srcBytes, dest }),
  pickFolder: () => invoke<string | null>("pick_folder"),
  openPath: (path: string) => invoke<boolean>("open_path", { path }),
  readLogTail: (path: string) => invoke<string>("read_log_tail", { path }),
  notify: (title: string, body: string) => invoke<void>("notify", { title, body }),
};

/** 订阅引擎事件（唯一 listener，由 App 顶层持有）。 */
export function onTaskEvent(cb: (event: TaskEvent) => void): Promise<UnlistenFn> {
  return listen<TaskEvent>(TASK_EVENT_CHANNEL, (ev) => cb(ev.payload));
}
