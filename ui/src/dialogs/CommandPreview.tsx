/** 命令行预览：只读展示 + 复制到剪贴板（对齐原版 CommandPreview）。 */

import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { ellipsizePath } from "../theme";
import type { TaskSnapshot } from "../types";
import { Dialog } from "../components/ui/Dialog";
import { useToast } from "../components/ui/Toast";

export function CommandPreview({
  task,
  onClose,
}: {
  task: TaskSnapshot;
  onClose: () => void;
}) {
  const toast = useToast();
  const [command, setCommand] = useState("加载中…");
  const timer = useRef<number>(0);

  useEffect(() => {
    // 300ms 防抖后拉取最新命令行
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      api.previewCommand(task).then(setCommand);
    }, 300);
    return () => window.clearTimeout(timer.current);
  }, [task]);

  const name = useMemo(() => task.name || "未命名任务", [task]);

  return (
    <Dialog open onClose={onClose} width={680}>
      <div className="flex flex-col gap-4 p-5">
        <div className="flex items-center gap-3">
          <span className="text-2xl text-[#06B6D4]">ℹ</span>
          <h2 className="flex-1 text-sm font-semibold text-foreground">
            任务「{name}」的 robocopy 命令
          </h2>
        </div>
        <div className="text-xs text-[#64748B]">
          {ellipsizePath(task.source)} <span className="text-[#2563EB]">→</span>{" "}
          {ellipsizePath(task.dest)}
        </div>
        <pre className="max-h-[240px] overflow-auto break-all whitespace-pre-wrap rounded-lg bg-[#0B1220] p-3.5 font-mono text-xs leading-relaxed text-[#93C5FD]">
          {command}
        </pre>
        <p className="text-[11px] leading-relaxed text-[#94A3B8]">
          提示：路径含空格时会自动加引号；该命令可直接粘贴到 CMD 中运行。试运行 /L
          只会列出将要复制的文件，不会写入任何数据。
        </p>
        <div className="flex justify-end gap-2.5">
          <button
            type="button"
            onClick={onClose}
            className="h-8 rounded-md border border-border bg-surface px-4 text-[13px] text-foreground transition-colors hover:bg-surfaceAlt"
          >
            关闭
          </button>
          <button
            type="button"
            onClick={() => {
              navigator.clipboard
                .writeText(command)
                .then(() => toast("success", "已复制到剪贴板"))
                .catch((e) => console.error(e));
            }}
            className="h-8 rounded-md bg-[#06B6D4] px-5 text-[13px] text-white transition-colors hover:bg-[#0891B2]"
          >
            复制到剪贴板
          </button>
        </div>
      </div>
    </Dialog>
  );
}
