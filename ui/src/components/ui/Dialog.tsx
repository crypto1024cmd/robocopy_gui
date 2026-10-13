/** 通用模态对话框：ESC 关闭、蒙层点击关闭、顶部偏上居中（对齐原版弹窗位置）。 */

import { useEffect, type ReactNode } from "react";

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  width?: number;
  /** 蒙层点击是否允许关闭（模态表单默认允许，与原版 ESC 行为一致） */
  children: ReactNode;
}

export function Dialog({ open, onClose, width = 520, children }: DialogProps) {
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[100] flex items-start justify-center bg-slate-950/45 pt-[13vh]"
      onMouseDown={onClose}
    >
      <div
        className="max-h-[78vh] overflow-hidden rounded-xl bg-card shadow-2xl ring-1 ring-border"
        style={{ width }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}
