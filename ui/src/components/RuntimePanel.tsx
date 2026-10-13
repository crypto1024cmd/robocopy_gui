/** 右侧栏：本次运行统计 + 运行设置 + 新任务默认参数（对齐原版 RuntimePanel）。 */

import { humanBytes } from "../theme";
import type { CopyOptions, TaskSnapshot } from "../types";
import { OptionsPanel } from "./OptionsPanel";

export interface RuntimePanelProps {
  tasks: TaskSnapshot[];
  parallel: number;
  pollInterval: number;
  defaultOptions: CopyOptions;
  onParallelChange: (n: number) => void;
  onPollIntervalChange: (s: number) => void;
  onDefaultOptionsChange: (o: CopyOptions) => void;
}

export function RuntimePanel({
  tasks,
  parallel,
  pollInterval,
  defaultOptions,
  onParallelChange,
  onPollIntervalChange,
  onDefaultOptionsChange,
}: RuntimePanelProps) {
  const done = tasks.filter((t) => t.status === "done").length;
  const failed = tasks.filter((t) => t.status === "failed").length;
  const copied = tasks.reduce((s, t) => s + t.copied_bytes, 0);
  const total = tasks.reduce((s, t) => s + t.total_bytes, 0);

  return (
    <div className="flex w-[340px] shrink-0 flex-col gap-3">
      {/* 本次运行 */}
      <div className="rounded-xl bg-card p-4 shadow-card ring-1 ring-border">
        <h3 className="mb-3 text-xs font-semibold text-foreground">本次运行</h3>
        <div className="flex flex-col gap-2 text-xs text-[#64748B]">
          <div className="flex justify-between">
            <span>
              共 {tasks.length} 个任务（已完成 {done} · 失败 {failed}）
            </span>
          </div>
          <div className="flex justify-between">
            <span>已复制</span>
            <span className="font-mono text-[#2563EB]">{humanBytes(copied)}</span>
          </div>
          <div className="flex justify-between">
            <span>总计</span>
            <span className="font-mono">{humanBytes(total)}</span>
          </div>
        </div>
      </div>

      {/* 运行设置 */}
      <div className="rounded-xl bg-card p-4 shadow-card ring-1 ring-border">
        <h3 className="mb-3 text-xs font-semibold text-foreground">运行设置</h3>
        <div className="flex flex-col gap-2.5 text-xs text-[#64748B]">
          <label className="flex items-center justify-between gap-2">
            <span>并行任务数</span>
            <select
              value={parallel}
              onChange={(e) => onParallelChange(Number(e.target.value))}
              className="h-[26px] rounded-md border border-[#CBD5E1] bg-surface px-2 text-xs outline-none"
            >
              {[1, 2, 3, 4, 5, 6, 7, 8].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center justify-between gap-2">
            <span>进度刷新</span>
            <select
              value={pollInterval}
              onChange={(e) => onPollIntervalChange(Number(e.target.value))}
              className="h-[26px] rounded-md border border-[#CBD5E1] bg-surface px-2 text-xs outline-none"
            >
              {[0.5, 1, 2, 3].map((s) => (
                <option key={s} value={s}>
                  {s} 秒
                </option>
              ))}
            </select>
          </label>
          <p className="text-[11px] leading-relaxed text-[#94A3B8]">
            轮询间隔越小进度越实时，但会占用少量磁盘 IO；并行任务数对机械硬盘建议不超过 2。
          </p>
        </div>
      </div>

      {/* 新任务默认参数 */}
      <div className="rounded-xl bg-card p-4 shadow-card ring-1 ring-border">
        <h3 className="mb-3 text-xs font-semibold text-foreground">新任务默认参数</h3>
        <OptionsPanel value={defaultOptions} onChange={onDefaultOptionsChange} />
      </div>
    </div>
  );
}
