/** 任务卡片：状态徽标、路径、进度、速度/ETA/统计与操作按钮（对齐原版 TaskCard）。 */

import { ellipsizePath, humanBytes, humanDuration, humanSpeed, summaryText } from "../theme";
import { isFinal, type TaskSnapshot } from "../types";
import { SmoothBar } from "./SmoothBar";
import { StatusBadge } from "./StatusBadge";

export interface TaskCardProps {
  task: TaskSnapshot;
  selected: boolean;
  isRunning: boolean;
  onSelect: () => void;
  onEdit: () => void;
  onPreview: () => void;
  onStop: () => void;
  onRerun: () => void;
  onLog: () => void;
  onRemove: () => void;
}

/** 小型操作按钮：hover 灰底，对齐原版 26px 小按钮。 */
function MiniButton({
  onClick,
  children,
  disabled = false,
  danger = false,
  title,
}: {
  onClick: () => void;
  children: string;
  disabled?: boolean;
  danger?: boolean;
  title?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`h-[26px] rounded-md px-2.5 text-xs transition-colors ${
        disabled
          ? "cursor-not-allowed text-slate-300"
          : danger
            ? "text-foreground hover:bg-surfaceAlt"
            : "text-foreground hover:bg-surfaceAlt"
      }`}
    >
      {children}
    </button>
  );
}

export function TaskCard({
  task,
  selected,
  isRunning,
  onSelect,
  onEdit,
  onPreview,
  onStop,
  onRerun,
  onLog,
  onRemove,
}: TaskCardProps) {
  const final = isFinal(task.status);
  const percent =
    task.total_bytes > 0
      ? Math.min(100, (task.copied_bytes * 100) / task.total_bytes)
      : final && !task.error
        ? 100
        : 0;

  // 第三行提示：失败红色优先，完成态显示退出码说明，否则显示阶段文本/参数摘要
  let line3: { text: string; danger: boolean } | null = null;
  if (task.status === "failed" && task.error) {
    line3 = { text: task.error, danger: true };
  } else if (task.status === "done" && task.rc_text) {
    line3 = { text: task.rc_text, danger: false };
  } else if (task.status === "stopped") {
    line3 = { text: task.error || "已停止", danger: false };
  } else if (task.phase_text) {
    line3 = { text: task.phase_text, danger: false };
  }

  const speed =
    task.speed_bps > 0 ? humanSpeed(task.speed_bps) : humanSpeed(task.avg_speed_bps);
  const current = task.current_file ? `  当前文件：${task.current_file}` : "";

  return (
    <div
      onClick={onSelect}
      className={`group relative flex cursor-pointer flex-col gap-2 rounded-xl bg-card p-4 shadow-card ring-1 transition-all duration-200 hover:shadow-lg ${
        selected ? "ring-2 ring-[#2563EB]" : "ring-border"
      }`}
    >
      {selected && (
        <div className="absolute inset-y-0 left-0 w-[3px] rounded-l-xl bg-[#2563EB]" />
      )}

      {/* 行1：操作 + 名称 + 徽标 */}
      <div className="flex items-center gap-1">
        <MiniButton
          onClick={onEdit}
          disabled={isRunning}
          title={isRunning ? "运行中不可编辑" : undefined}
        >
          编辑
        </MiniButton>
        <MiniButton onClick={onPreview}>预览</MiniButton>
        <span className="ml-1 truncate text-[13px] font-semibold text-foreground">
          {task.name || "未命名任务"}
        </span>
        <div className="ml-auto flex items-center gap-1">
          <StatusBadge status={task.status} />
        </div>
      </div>

      {/* 行2：源 → 目标 */}
      <div className="flex items-center gap-1 text-xs text-[#64748B]">
        <span className="truncate font-mono" title={task.source}>
          {ellipsizePath(task.source)}
        </span>
        <span className="shrink-0 text-[#2563EB]">→</span>
        <span className="truncate font-mono" title={task.dest}>
          {ellipsizePath(task.dest)}
        </span>
      </div>

      {/* 行3：阶段/错误/退出码 */}
      <div className={`truncate text-xs ${line3?.danger ? "text-[#EF4444]" : "text-[#64748B]"}`}>
        {line3?.danger ? (
          <span className="inline-flex items-center gap-1">⚠ {line3.text}</span>
        ) : line3 ? (
          line3.text
        ) : (
          <span className="text-[#94A3B8]">{summaryText(task.options)}</span>
        )}
      </div>

      {/* 行4：进度条 + 百分比 + 字节 */}
      <div className="flex items-center gap-3">
        <SmoothBar value={percent} height={8} className="flex-1" />
        <span className="w-[58px] shrink-0 text-right font-mono text-xs text-[#0F172A]">
          {percent.toFixed(1)}%
        </span>
        <span className="shrink-0 font-mono text-xs text-[#64748B]">
          {humanBytes(task.copied_bytes)} / {humanBytes(task.total_bytes)}
        </span>
      </div>

      {/* 行5：速度 / ETA / 当前文件 */}
      <div className="flex items-center gap-2 truncate text-xs text-[#64748B]">
        <span className="shrink-0 font-mono text-[#2563EB]">{speed}</span>
        <span className="shrink-0">ETA {humanDuration(task.eta_seconds)}</span>
        {current && <span className="truncate">{current}</span>}
      </div>

      {/* 行6：统计 + 右侧操作 */}
      <div className="flex items-center gap-2 text-xs text-[#94A3B8]">
        <span className="truncate">
          {`共 ${task.total_files} 文件`}
          {task.already_bytes > 0 && ` · 已存在 ${humanBytes(task.already_bytes)}`}
          {task.verify_checked > 0 && ` · 校验 ${task.verify_passed}/${task.verify_checked}`}
          {` · 耗时 ${humanDuration(task.elapsed)}`}
        </span>
        <div className="ml-auto flex shrink-0 items-center gap-0.5">
          <MiniButton
            onClick={onStop}
            disabled={!isRunning || final}
            danger
            title={final || !isRunning ? undefined : "停止该任务"}
          >
            停止
          </MiniButton>
          <MiniButton onClick={onRerun} disabled={isRunning || !final}>
            重跑
          </MiniButton>
          <MiniButton onClick={onLog} disabled={!task.log_path}>
            日志
          </MiniButton>
          <MiniButton onClick={onRemove} disabled={isRunning} danger>
            删除
          </MiniButton>
        </div>
      </div>
    </div>
  );
}
