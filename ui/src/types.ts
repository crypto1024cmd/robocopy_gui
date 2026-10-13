/** 与 rc-core serde 序列化契约对齐的 TypeScript 类型（snake_case 字段名）。 */

export type TaskStatus =
  | "pending"
  | "scanning"
  | "running"
  | "verifying"
  | "done"
  | "stopped"
  | "failed";

export type VerifyLevel = "none" | "quick" | "strict";
export type HashAlgo = "md5" | "sha1" | "sha256";

export interface CopyOptions {
  threads: number;
  mirror: boolean;
  dry_run: boolean;
  verify_level: VerifyLevel;
  hash_algo: HashAlgo;
  retries: number;
  wait_seconds: number;
  restartable: boolean;
}

export interface VerifyDiff {
  path: string;
  reason: string;
}

/** CopyTask 快照（UI 拿到的永远是副本，绝不直接读运行中的对象）。 */
export interface TaskSnapshot {
  source: string;
  dest: string;
  name: string;
  options: CopyOptions;
  task_id: string;
  status: TaskStatus;
  phase_text: string;
  total_bytes: number;
  copied_bytes: number;
  total_files: number;
  copied_files: number;
  already_bytes: number;
  speed_bps: number;
  avg_speed_bps: number;
  eta_seconds: number | null;
  current_file: string;
  returncode: number | null;
  rc_text: string;
  log_path: string;
  error: string;
  verify_checked: number;
  verify_passed: number;
  verify_failed: VerifyDiff[];
  skipped_count: number;
  started_at: number;
  finished_at: number;
  elapsed: number;
}

/** 引擎 → UI 统一事件（serde tag = "kind", snake_case）。 */
export type TaskEvent =
  | { kind: "updated"; task: TaskSnapshot }
  | { kind: "log"; task_id: string; line: string }
  | { kind: "removed"; task_id: string }
  /** 引擎调度运行状态变化；「开始/停止」按钮据此驱动，不能用任务状态推断 */
  | { kind: "running"; running: boolean }
  | { kind: "all_done" }
  | { kind: "error"; message: string };

export interface DriveInfo {
  root: string;
  label: string;
  fs: string;
  total: number;
  free: number;
}

export type PathRelation =
  | "ok"
  | "same"
  | "dest_in_src"
  | "src_in_dest"
  | "empty"
  | "missing";

export const RELATION_MESSAGES: Record<PathRelation, string> = {
  ok: "",
  empty: "源路径与目标目录都必须填写",
  same: "源路径与目标目录不能相同",
  dest_in_src: "目标目录位于源目录内部，会导致自我递归复制，已阻止",
  src_in_dest: "源位于目标目录内部，可能造成数据覆盖，已阻止",
  missing: "源路径不存在（既不是文件夹也不是文件）",
};

export interface ScanEstimate {
  files: number;
  dirs: number;
  total_bytes: number;
  /** 源是否为单个文件（据此禁用 /MIR 并调整文案） */
  is_file: boolean;
}

export interface DiskFreeHint {
  ok: boolean;
  text: string;
}

export function defaultOptions(): CopyOptions {
  return {
    threads: 16,
    mirror: false,
    dry_run: false,
    verify_level: "quick",
    hash_algo: "md5",
    retries: 1,
    wait_seconds: 1,
    restartable: false,
  };
}

export function makeEmptyTask(overrides?: Partial<TaskSnapshot>): TaskSnapshot {
  const base: TaskSnapshot = {
    source: "",
    dest: "",
    name: "",
    options: defaultOptions(),
    task_id: "",
    status: "pending",
    phase_text: "",
    total_bytes: 0,
    copied_bytes: 0,
    total_files: 0,
    copied_files: 0,
    already_bytes: 0,
    speed_bps: 0,
    avg_speed_bps: 0,
    eta_seconds: null,
    current_file: "",
    returncode: null,
    rc_text: "",
    log_path: "",
    error: "",
    verify_checked: 0,
    verify_passed: 0,
    verify_failed: [],
    skipped_count: 0,
    started_at: 0,
    finished_at: 0,
    elapsed: 0,
  };
  return {
    ...base,
    ...overrides,
    options: { ...base.options, ...overrides?.options },
  };
}

export function isFinal(status: TaskStatus): boolean {
  return status === "done" || status === "stopped" || status === "failed";
}
