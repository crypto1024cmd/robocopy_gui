/** 深色控制台日志面板：按任务过滤、自动滚动、折叠/展开。 */

import { useEffect, useRef, useState } from "react";

export interface LogConsoleProps {
  /** 每行格式 "task_id \t line"，由 App 层组装 */
  lines: string[];
  /** 日志缓冲区被裁剪的行数（性能护栏提示） */
  droppedCount: number;
  /** 任务下拉选项（id + 显示名） */
  tasks: { id: string; name: string }[];
}

const MAX_HEIGHT = 260;

export function LogConsole({ lines, droppedCount, tasks }: LogConsoleProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [taskId, setTaskId] = useState<string>("__all__");
  const boxRef = useRef<HTMLDivElement>(null);

  // 按任务过滤（每行格式 "task_id \t line" 由 App 层组装）
  const visible = taskId === "__all__" ? lines : lines.filter((l) => l.startsWith(taskId));
  const shown = visible.slice(-800);

  useEffect(() => {
    if (autoScroll && boxRef.current) {
      boxRef.current.scrollTop = boxRef.current.scrollHeight;
    }
  }, [shown.length, autoScroll]);

  const header = (
    <div className="flex h-[38px] shrink-0 items-center gap-3 border-t border-[#1E293B] bg-[#0B1220] px-4">
      <span className="text-xs font-medium text-[#CBD5E1]">运行日志</span>
      <select
        value={taskId}
        onChange={(e) => setTaskId(e.target.value)}
        className="h-[22px] rounded border border-[#1E293B] bg-[#0F172A] px-1.5 text-[11px] text-[#CBD5E1] outline-none"
      >
        <option value="__all__">全部任务</option>
        {tasks.map((t) => (
          <option key={t.id} value={t.id}>
            {t.name}
          </option>
        ))}
      </select>
      <label className="flex cursor-pointer select-none items-center gap-1 text-[11px] text-[#94A3B8]">
        <input
          type="checkbox"
          checked={autoScroll}
          onChange={(e) => setAutoScroll(e.target.checked)}
          className="h-3 w-3 accent-[#2563EB]"
        />
        自动滚动
      </label>
      <span className="text-[11px] text-[#475569]">
        {visible.length} 行
        {droppedCount > 0 && `（已裁剪 ${droppedCount} 行）`}
      </span>
      <button
        type="button"
        onClick={() => setCollapsed((c) => !c)}
        className="ml-auto text-[11px] text-[#94A3B8] transition-colors hover:text-white"
      >
        {collapsed ? "展开 ▲" : "收起 ▼"}
      </button>
    </div>
  );

  return (
    <div className="flex shrink-0 flex-col bg-[#0B1220]">
      {header}
      {!collapsed && (
        <div
          ref={boxRef}
          className="console-scroll px-4 pb-3 font-mono text-[11px] leading-[1.7] text-[#CBD5E1]"
          style={{ height: MAX_HEIGHT, overflowY: "auto" }}
        >
          {shown.length === 0 ? (
            <div className="pt-6 text-center text-[#475569]">暂无日志输出</div>
          ) : (
            shown.map((line, i) => {
              const body = line.indexOf("\t") >= 0 ? line.slice(line.indexOf("\t") + 1) : line;
              const isOwn = body.startsWith("»");
              return (
                <div key={i} className="whitespace-pre-wrap break-all">
                  {isOwn ? <span className="text-[#93C5FD]">{body}</span> : body}
                </div>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}

export const LOG_BUFFER_LIMIT = 2000;
export const LOG_DROP_STEP = 400;
