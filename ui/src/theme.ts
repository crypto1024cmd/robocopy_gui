/** 主题常量与格式化工具 —— 对齐原版 widgets.py 的 COLORS / STATUS_COLORS。 */

import type { TaskStatus } from "./types";

/** 关键配色（与原版 COLORS 一致）。 */
export const COLORS = {
  bg: "#F8FAFC",
  surface: "#FFFFFF",
  surfaceAlt: "#F1F5F9",
  hover: "#F8FAFC",
  border: "#E2E8F0",
  borderStrong: "#CBD5E1",
  topbar: "#0F172A",
  topbarAlt: "#1E293B",
  text: "#0F172A",
  textDim: "#64748B",
  textFaint: "#94A3B8",
  onDark: "#FFFFFF",
  onDarkDim: "#94A3B8",
  primary: "#2563EB",
  primaryLight: "#3B82F6",
  primaryDark: "#1D4ED8",
  success: "#10B981",
  warning: "#F59E0B",
  danger: "#EF4444",
  info: "#06B6D4",
  consoleBg: "#0B1220",
  consoleFg: "#CBD5E1",
  previewFg: "#93C5FD",
} as const;

/** 状态徽标颜色映射。 */
export const STATUS_COLORS: Record<TaskStatus, string> = {
  pending: "#94A3B8",
  scanning: "#F59E0B",
  running: "#2563EB",
  verifying: "#06B6D4",
  done: "#10B981",
  stopped: "#F59E0B",
  failed: "#EF4444",
};

/** 状态展示文案（与后端 label() 一致）。 */
export const STATUS_LABELS: Record<TaskStatus, string> = {
  pending: "待执行",
  scanning: "扫描中",
  running: "复制中",
  verifying: "校验中",
  done: "已完成",
  stopped: "已停止",
  failed: "失败",
};

/** 十六进制颜色混合：ratio 为混入 other 的比例。 */
export function mixHex(hex: string, other: string, ratio: number): string {
  const a = parseInt(hex.slice(1), 16);
  const b = parseInt(other.slice(1), 16);
  const ch = (shift: number) => {
    const ca = (a >> shift) & 0xff;
    const cb = (b >> shift) & 0xff;
    return Math.round(ca + (cb - ca) * ratio);
  };
  const r = ch(16);
  const g = ch(8);
  const bl = ch(0);
  const num = (r << 16) | (g << 8) | bl;
  return `#${num.toString(16).padStart(6, "0")}`;
}

/** 压暗（混入黑色）。 */
export function darken(hex: string, ratio: number): string {
  return mixHex(hex, "#000000", ratio);
}

/** 提亮（混入白色）。 */
export function lighten(hex: string, ratio: number): string {
  return mixHex(hex, "#FFFFFF", ratio);
}

/** 字节数转人类可读文本（对齐 winfs.human_bytes）。 */
export function humanBytes(size: number, precision = 1): string {
  let value = size;
  for (const unit of ["B", "KB", "MB", "GB", "TB"]) {
    if (Math.abs(value) < 1024) {
      if (unit === "B") return `${Math.trunc(value)} B`;
      return `${value.toFixed(precision)} ${unit}`;
    }
    value /= 1024;
  }
  return `${value.toFixed(precision)} PB`;
}

/** 速度格式化，如 "123.4 MB/s"。 */
export function humanSpeed(bps: number): string {
  if (bps <= 0) return "-- /s";
  return `${humanBytes(bps)}/s`;
}

/** 时长格式化，如 "1 时 02 分" / "3 分 12 秒" / "45 秒"。 */
export function humanDuration(seconds: number | null): string {
  if (seconds === null || seconds < 0) return "--";
  const total = Math.trunc(seconds);
  if (total < 60) return `${total} 秒`;
  const minutes = Math.trunc(total / 60);
  const sec = total % 60;
  if (minutes < 60) return `${minutes} 分 ${String(sec).padStart(2, "0")} 秒`;
  const hours = Math.trunc(minutes / 60);
  const minute = minutes % 60;
  return `${hours} 时 ${String(minute).padStart(2, "0")} 分`;
}

/** 百分比固定格式：` 12.3%`（5 字符右对齐，对齐原版格式化口径）。 */
export function percentText(percent: number): string {
  return `${percent.toFixed(1).padStart(5, " ")}%`;
}

/** 复制参数一行摘要（对齐 summary_text）。 */
export function summaryText(options: {
  mirror: boolean;
  dry_run: boolean;
  restartable: boolean;
  threads: number;
  verify_level: string;
}): string {
  let mode = options.mirror ? "镜像 /MIR" : "普通 /E";
  if (options.dry_run) mode += " · 试运行 /L";
  if (options.restartable) mode += " · 重启模式 /Z";
  const verifyLabel =
    options.verify_level === "none"
      ? "不校验"
      : options.verify_level === "quick"
        ? "大小+时间戳"
        : "哈希校验";
  return `${mode} · /MT:${options.threads} · 校验 ${verifyLabel}`;
}

/** 路径超长截断：保留首段与末两段，中间以 `…` 折叠。 */
export function ellipsizePath(path: string, maxLen = 72): string {
  if (path.length <= maxLen) return path;
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(sep);
  if (parts.length <= 3) return path;
  const head = parts.slice(0, 2).join(sep);
  const tail = parts.slice(-2).join(sep);
  const out = `${head}${sep}…${sep}${tail}`;
  return out.length < path.length ? out : path;
}
