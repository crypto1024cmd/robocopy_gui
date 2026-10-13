/** 确认对话框（对齐原版 ConfirmDialog 的视觉与文案结构）。 */

import { Dialog } from "./Dialog";

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  /** 支持 \n 换行的正文 */
  body: string;
  okText: string;
  danger?: boolean;
  info?: boolean;
  onCancel: () => void;
  onOk: () => void;
}

const ICON = (danger?: boolean) =>
  danger ? (
    <span className="text-2xl text-[#EF4444]">⚠</span>
  ) : (
    <span className="text-2xl text-[#06B6D4]">ℹ</span>
  );

export function ConfirmDialog({
  open,
  title,
  body,
  okText,
  danger = false,
  info = false,
  onCancel,
  onOk,
}: ConfirmDialogProps) {
  const okClass = danger
    ? "bg-[#EF4444] text-white hover:bg-[#DC2626]"
    : info
      ? "bg-[#06B6D4] text-white hover:bg-[#0891B2]"
      : "bg-primary text-primary-foreground hover:bg-[#1D4ED8]";

  return (
    <Dialog open={open} onClose={onCancel} width={520}>
      <div className="flex flex-col gap-4 p-5">
        <div className="flex items-center gap-3">
          {ICON(danger)}
          <h2 className="flex-1 text-sm font-semibold text-foreground">{title}</h2>
        </div>
        <p className="whitespace-pre-line pl-9 text-[13px] leading-relaxed text-[#64748B]">
          {body}
        </p>
        <div className="flex justify-end gap-2.5">
          <button
            type="button"
            onClick={onCancel}
            className="h-8 rounded-md border border-border bg-surface px-4 text-[13px] text-foreground transition-colors hover:bg-surfaceAlt"
          >
            取消
          </button>
          <button
            type="button"
            onClick={onOk}
            className={`h-8 rounded-md px-5 text-[13px] transition-colors ${okClass}`}
          >
            {okText}
          </button>
        </div>
      </div>
    </Dialog>
  );
}
