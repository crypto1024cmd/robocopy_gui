/** 日志查看器：打开任务日志尾部 32KB 只读展示（对齐原版 LogViewer）。 */

import { useEffect, useState } from "react";
import { api } from "../api";
import { Dialog } from "../components/ui/Dialog";
import type { TaskSnapshot } from "../types";

export function LogViewer({ task, onClose }: { task: TaskSnapshot; onClose: () => void }) {
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    setContent(null);
    setError("");
    api.readLogTail(task.log_path).then(setContent).catch((e) => setError(String(e)));
  }, [task.log_path]);

  const name = task.name || "未命名任务";

  return (
    <Dialog open onClose={onClose} width={820}>
      <div className="flex h-[520px] flex-col gap-3 p-5">
        <div className="flex items-center gap-2">
          <h2 className="flex-1 truncate text-sm font-semibold text-foreground">
            任务「{name}」日志
          </h2>
          <span className="shrink-0 font-mono text-[11px] text-[#94A3B8]">
            {task.log_path}
          </span>
        </div>
        <div className="console-scroll min-h-0 flex-1 overflow-auto rounded-lg bg-[#0B1220] p-3.5">
          {error ? (
            <div className="p-4 text-center text-xs text-[#EF4444]">日志读取失败：{error}</div>
          ) : content === null ? (
            <div className="p-4 text-center text-xs text-[#475569]">正在读取日志…</div>
          ) : content.trim().length === 0 ? (
            <div className="p-4 text-center text-xs text-[#475569]">日志为空</div>
          ) : (
            <pre className="break-all whitespace-pre-wrap font-mono text-[11px] leading-[1.7] text-[#CBD5E1]">
              {content}
            </pre>
          )}
        </div>
        <p className="text-[11px] text-[#94A3B8]">
          完整日志已落盘（UTF-8），此处展示文件尾部约 32KB，滚动查看任务全程记录。
        </p>
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
