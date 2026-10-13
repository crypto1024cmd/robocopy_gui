/** 设置对话框：日志目录入口、清空已完成任务与版本信息（对齐原版 SettingsDialog）。 */

import { Dialog } from "../components/ui/Dialog";
import { api } from "../api";

export interface SettingsDialogProps {
  open: boolean;
  onClose: () => void;
  onClearFinished: () => void;
}

export function SettingsDialog({ open, onClose, onClearFinished }: SettingsDialogProps) {
  if (!open) return null;
  return (
    <Dialog open={open} onClose={onClose} width={480}>
      <div className="flex flex-col gap-4 p-5">
        <h2 className="text-sm font-semibold text-foreground">设置</h2>

        <div className="flex flex-col divide-y divide-border rounded-lg ring-1 ring-border">
          <div className="flex items-center justify-between gap-3 px-4 py-3">
            <div>
              <div className="text-[13px] text-foreground">打开日志目录</div>
              <div className="mt-0.5 text-[11px] text-[#94A3B8]">
                每次任务的完整日志都按时间戳落盘在该目录
              </div>
            </div>
            <button
              type="button"
              onClick={() => api.openPath("logs")}
              className="h-8 shrink-0 rounded-md bg-primary px-4 text-xs text-primary-foreground transition-colors hover:bg-[#1D4ED8]"
            >
              打开
            </button>
          </div>

          <div className="flex items-center justify-between gap-3 px-4 py-3">
            <div>
              <div className="text-[13px] text-foreground">清空已完成任务</div>
              <div className="mt-0.5 text-[11px] text-[#94A3B8]">
                把已完成/已停止/失败的任务从列表移除，日志与文件不受影响
              </div>
            </div>
            <button
              type="button"
              onClick={onClearFinished}
              className="h-8 shrink-0 rounded-md border border-[#FCA5A5] bg-surface px-4 text-xs text-[#DC2626] transition-colors hover:bg-[#FEF2F2]"
            >
              清空
            </button>
          </div>
        </div>

        <div className="rounded-lg bg-surfaceAlt p-3 text-[11px] leading-relaxed text-[#94A3B8]">
          <div>
            版本 <span className="font-medium text-[#64748B]">v2.0（Rust + Tauri 重构版）</span>
          </div>
          <div className="mt-1">
            复制由 Windows 自带的 robocopy.exe（/MT 多线程）完成；本程序负责调度、
            实时进度与复制后校验。日志按任务完整落盘，UI 侧节流展示。
          </div>
        </div>

        <div className="flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="h-8 rounded-md bg-primary px-5 text-[13px] text-primary-foreground transition-colors hover:bg-[#1D4ED8]"
          >
            关闭
          </button>
        </div>
      </div>
    </Dialog>
  );
}
