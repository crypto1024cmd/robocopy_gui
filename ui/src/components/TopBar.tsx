/** 顶栏：标题、总进度、右侧按钮组（对齐原版 62px 深色 TopBar）。 */

import type { ReactNode } from "react";
import { humanBytes, humanSpeed } from "../theme";
import { SmoothBar } from "./SmoothBar";

export interface TopBarProps {
  /** 全部任务 copied_bytes 之和 */
  copiedBytes: number;
  /** 全部任务 total_bytes 之和 */
  totalBytes: number;
  /** 瞬时速度：全部非终态任务 speed_bps 之和 */
  speedBps: number;
  /** 非终态任务数 */
  activeCount: number;
  taskCount: number;
  isRunning: boolean;
  showSettings: boolean;
  onStartAll: () => void;
  onStopAll: () => void;
  onAdd: () => void;
  onSettings: () => void;
  onOpenLogs: () => void;
}

/** 顶栏按钮（34px 高，对齐原版 TopBarButton）。 */
function TopBarButton({
  onClick,
  children,
  disabled = false,
  kind = "ghost",
  title,
}: {
  onClick: () => void;
  children: ReactNode;
  disabled?: boolean;
  kind?: "primary" | "danger" | "success" | "ghost";
  title?: string;
}) {
  const base =
    "h-[34px] rounded-md px-4 text-[13px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40";
  const kindClass =
    kind === "primary"
      ? "bg-[#2563EB] text-white hover:bg-[#3B82F6]"
      : kind === "danger"
        ? "bg-[#EF4444] text-white hover:bg-[#DC2626]"
        : kind === "success"
          ? "bg-[#10B981] text-white hover:bg-[#059669]"
          : "text-[#CBD5E1] hover:bg-[#334155] hover:text-white";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`${base} ${kindClass}`}
    >
      {children}
    </button>
  );
}

export function TopBar({
  copiedBytes,
  totalBytes,
  speedBps,
  activeCount,
  taskCount,
  isRunning,
  showSettings,
  onStartAll,
  onStopAll,
  onAdd,
  onSettings,
  onOpenLogs,
}: TopBarProps) {
  const percent = totalBytes > 0 ? Math.min(100, (copiedBytes * 100) / totalBytes) : 0;

  return (
    <div className="flex h-[62px] shrink-0 items-center gap-5 bg-[#0F172A] px-6">
      {/* 标题 */}
      <div className="flex shrink-0 items-center gap-2.5">
        <span className="text-lg text-[#3B82F6]">⇉</span>
        <h1 className="text-[15px] font-semibold text-white">RoboCopy 批量搬运</h1>
        <span className="rounded bg-[#334155] px-1.5 py-0.5 text-[10px] font-medium text-[#93C5FD]">
          MT
        </span>
      </div>

      {/* 总进度 */}
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <SmoothBar
          value={percent}
          height={8}
          trackClass="bg-[#1E293B]"
          className="min-w-[120px] max-w-[260px]"
        />
        <span className="shrink-0 whitespace-nowrap font-mono text-xs text-[#CBD5E1]">
          {humanBytes(copiedBytes)} / {humanBytes(totalBytes)}
          <span className="ml-2 text-[#94A3B8]">（{percent.toFixed(1)}%）</span>
        </span>
        <span className="shrink-0 whitespace-nowrap font-mono text-xs text-[#93C5FD]">
          {humanSpeed(speedBps)}
        </span>
        <span className="shrink-0 whitespace-nowrap text-xs text-[#94A3B8]">
          进行中 {activeCount}/{taskCount}
        </span>
      </div>

      {/* 按钮组 */}
      <div className="flex shrink-0 items-center gap-2">
        <TopBarButton kind="primary" onClick={onStartAll} disabled={isRunning || taskCount === 0}>
          ▶ 开始全部
        </TopBarButton>
        <TopBarButton kind="danger" onClick={onStopAll} disabled={!isRunning}>
          ■ 停止全部
        </TopBarButton>
        <TopBarButton onClick={onAdd}>＋ 新增任务</TopBarButton>
        {showSettings && <TopBarButton onClick={onSettings}>⚙ 设置</TopBarButton>}
        <TopBarButton onClick={onOpenLogs} title="打开日志目录">
          🗀 日志目录
        </TopBarButton>
      </div>
    </div>
  );
}
