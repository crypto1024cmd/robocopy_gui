/** 状态彩色徽标（浅底 + 中边框 + 深文字，对齐原版配色算法）。 */

import { STATUS_COLORS, STATUS_LABELS, darken, lighten } from "../theme";
import type { TaskStatus } from "../types";

export function StatusBadge({ status }: { status: TaskStatus }) {
  const color = STATUS_COLORS[status];
  const bg = lighten(color, 0.86);
  const border = lighten(color, 0.55);
  const fg = darken(color, 0.15);
  return (
    <span
      className="inline-flex shrink-0 items-center rounded-full border px-2.5 py-[3px] text-[11px] font-medium leading-none"
      style={{ backgroundColor: bg, borderColor: border, color: fg }}
    >
      {STATUS_LABELS[status]}
    </span>
  );
}
