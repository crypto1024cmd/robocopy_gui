/** 主界面装配：状态管理、事件订阅、日志缓冲与各对话框调度。 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, onTaskEvent } from "./api";
import { EmptyState } from "./components/EmptyState";
import { LogConsole, LOG_BUFFER_LIMIT, LOG_DROP_STEP } from "./components/LogConsole";
import { RuntimePanel } from "./components/RuntimePanel";
import { TaskCard } from "./components/TaskCard";
import { TopBar } from "./components/TopBar";
import { ConfirmDialog } from "./components/ui/ConfirmDialog";
import { ToastProvider, useToast } from "./components/ui/Toast";
import { CommandPreview } from "./dialogs/CommandPreview";
import { LogViewer } from "./dialogs/LogViewer";
import { SettingsDialog } from "./dialogs/SettingsDialog";
import { TaskEditor } from "./dialogs/TaskEditor";
import { defaultOptions, isFinal, type CopyOptions, type TaskEvent, type TaskSnapshot, type TaskStatus } from "./types";

interface ConfirmSpec {
  title: string;
  body: string;
  okText: string;
  danger?: boolean;
  info?: boolean;
  onOk: () => void;
}

function AppInner() {
  const toast = useToast();
  const [tasks, setTasks] = useState<Record<string, TaskSnapshot>>({});
  const [isRunning, setIsRunning] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [parallel, setParallel] = useState(1);
  const [pollInterval, setPollInterval] = useState(1);
  const [defaultOpts, setDefaultOpts] = useState<CopyOptions>(defaultOptions);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [dropped, setDropped] = useState(0);
  const [winWidth, setWinWidth] = useState(1280);

  const logBuffer = useRef<string[]>([]);
  const prevStatus = useRef<Record<string, TaskStatus>>({});
  /** 任务真源（事件按到达顺序同步维护，避免 React 批量更新造成判定滞后） */
  const tasksRef = useRef<Record<string, TaskSnapshot>>({});

  // 对话框状态
  const [editorTask, setEditorTask] = useState<TaskSnapshot | null | undefined>(undefined);
  const [previewTask, setPreviewTask] = useState<TaskSnapshot | null>(null);
  const [logTask, setLogTask] = useState<TaskSnapshot | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmSpec | null>(null);

  const askConfirm = useCallback((spec: ConfirmSpec) => setConfirm(spec), []);

  // ---- 初始化 + 事件订阅 ----
  useEffect(() => {
    api.listTasks().then((list) => {
      const map: Record<string, TaskSnapshot> = {};
      for (const t of list) {
        map[t.task_id] = t;
        prevStatus.current[t.task_id] = t.status;
      }
      tasksRef.current = map;
      setTasks(map);
      setSelectedId(list[0]?.task_id ?? null);
    });
    api.isRunning().then(setIsRunning);
    const un = onTaskEvent((event: TaskEvent) => {
      switch (event.kind) {
        case "updated": {
          const task = event.task;
          const before = prevStatus.current[task.task_id];
          prevStatus.current[task.task_id] = task.status;
          const next = { ...tasksRef.current, [task.task_id]: task };
          tasksRef.current = next;
          setTasks(next);
          // 注意：不能用「有无非终态任务」推断 isRunning —— 待执行(PENDING)
          // 的任务并不代表引擎在跑，否则刚添加完任务就会把「开始」按钮禁用。
          // 运行状态只由后端 running / all_done 事件驱动。
          if (before && !isFinal(before) && isFinal(task.status)) {
            const name = task.name || "未命名任务";
            if (task.status === "done") {
              toast("success", `任务「${name}」完成`);
            } else if (task.status === "failed") {
              toast("error", `任务「${name}」失败：${task.error || task.rc_text || "未知错误"}`);
            } else if (task.status === "stopped") {
              toast("warning", `任务「${name}」已停止`);
            }
          }
          break;
        }
        case "log":
          logBuffer.current.push(`${event.task_id}\t${event.line}`);
          break;
        case "removed": {
          const next = { ...tasksRef.current };
          delete next[event.task_id];
          tasksRef.current = next;
          setTasks(next);
          setSelectedId((cur) => (cur === event.task_id ? null : cur));
          break;
        }
        case "running":
          setIsRunning(event.running);
          break;
        case "all_done":
          setIsRunning(false);
          toast("info", "所有任务已执行完毕");
          break;
        case "error":
          toast("error", event.message);
          break;
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [toast]);

  // ---- 日志缓冲：60ms 批量刷新（对齐原版节流节奏） ----
  useEffect(() => {
    const timer = window.setInterval(() => {
      if (logBuffer.current.length === 0) return;
      const chunk = logBuffer.current;
      logBuffer.current = [];
      setLogLines((prev) => {
        let merged = [...prev, ...chunk];
        if (merged.length > LOG_BUFFER_LIMIT) {
          const drop =
            Math.ceil((merged.length - LOG_BUFFER_LIMIT) / LOG_DROP_STEP) * LOG_DROP_STEP;
          merged = merged.slice(drop);
          setDropped((d) => d + drop);
        }
        return merged;
      });
    }, 60);
    return () => window.clearInterval(timer);
  }, []);

  // ---- 响应式：窄窗口隐藏右栏与设置按钮 ----
  useEffect(() => {
    const onResize = () => setWinWidth(window.innerWidth);
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const taskList = useMemo(
    () => Object.values(tasks).sort((a, b) => a.source.localeCompare(b.source)),
    [tasks],
  );

  const totals = useMemo(() => {
    let copied = 0;
    let total = 0;
    let speed = 0;
    let active = 0;
    for (const t of taskList) {
      copied += t.copied_bytes;
      total += t.total_bytes;
      if (!isFinal(t.status)) {
        speed += t.speed_bps;
        active += 1;
      }
    }
    return { copied, total, speed, active };
  }, [taskList]);

  // ---- 操作 ----
  const startAll = useCallback(() => {
    api.startAll(parallel).then((ok) => {
      if (ok) setIsRunning(true);
    });
  }, [parallel]);

  const stopAll = useCallback(() => {
    askConfirm({
      title: "停止所有任务？",
      body: "运行中的 robocopy 进程将被立即终止，目标目录中可能留下不完整文件。",
      okText: "停止",
      danger: true,
      onOk: () => api.stopAll(),
    });
  }, [askConfirm]);

  const stopTask = useCallback(
    (id: string) => {
      const name = tasks[id]?.name || "未命名任务";
      askConfirm({
        title: `停止任务「${name}」？`,
        body: "运行中的 robocopy 进程将被立即终止，目标目录中可能留下不完整文件。",
        okText: "停止任务",
        danger: true,
        onOk: () => api.stopTask(id),
      });
    },
    [tasks, askConfirm],
  );

  const rerunTask = useCallback(
    (id: string) => {
      api.resetTask(id).then((ok) => {
        if (ok) {
          const name = tasks[id]?.name || "未命名任务";
          toast("info", `任务「${name}」已重置为待执行`);
        }
      });
    },
    [tasks, toast],
  );

  const removeTask = useCallback(
    (id: string) => {
      const name = tasks[id]?.name || "未命名任务";
      askConfirm({
        title: `删除任务「${name}」？`,
        body: "该操作只把它从列表移除，已复制的文件不受影响。",
        okText: "删除",
        danger: true,
        onOk: () => {
          api.removeTask(id);
        },
      });
    },
    [tasks, askConfirm],
  );

  // ---- 快捷键：Ctrl+N 新增 / F5 开始全部 / Ctrl+L 日志目录 ----
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const key = e.key.toLowerCase();
      if (e.ctrlKey && key === "n") {
        e.preventDefault();
        setEditorTask(null);
      } else if (e.key === "F5") {
        e.preventDefault();
        if (!isRunning && taskList.length > 0) startAll();
      } else if (e.ctrlKey && key === "l") {
        e.preventDefault();
        api.openPath("logs");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isRunning, taskList.length, startAll]);

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background">
      <TopBar
        copiedBytes={totals.copied}
        totalBytes={totals.total}
        speedBps={totals.speed}
        activeCount={totals.active}
        taskCount={taskList.length}
        isRunning={isRunning}
        showSettings={winWidth >= 1120}
        onStartAll={startAll}
        onStopAll={stopAll}
        onAdd={() => setEditorTask(null)}
        onSettings={() => setSettingsOpen(true)}
        onOpenLogs={() => api.openPath("logs")}
      />
      <div className="h-[3px] shrink-0 bg-gradient-to-r from-[#2563EB] to-[#06B6D4]" />

      <div className="flex min-h-0 flex-1 gap-4 p-4">
        <div className="console-scroll min-w-0 flex-1 overflow-y-auto">
          {taskList.length === 0 ? (
            <EmptyState onAdd={() => setEditorTask(null)} />
          ) : (
            <div className="flex flex-col gap-3">
              {taskList.map((t) => (
                <TaskCard
                  key={t.task_id}
                  task={t}
                  selected={selectedId === t.task_id}
                  isRunning={isRunning}
                  onSelect={() => setSelectedId(t.task_id)}
                  onEdit={() => setEditorTask(t)}
                  onPreview={() => setPreviewTask(t)}
                  onStop={() => stopTask(t.task_id)}
                  onRerun={() => rerunTask(t.task_id)}
                  onLog={() => setLogTask(t)}
                  onRemove={() => removeTask(t.task_id)}
                />
              ))}
            </div>
          )}
        </div>
        {winWidth >= 1040 && (
          <RuntimePanel
            tasks={taskList}
            parallel={parallel}
            pollInterval={pollInterval}
            defaultOptions={defaultOpts}
            onParallelChange={setParallel}
            onPollIntervalChange={(s) => {
              setPollInterval(s);
              api.setPollInterval(s);
            }}
            onDefaultOptionsChange={setDefaultOpts}
          />
        )}
      </div>

      <LogConsole
        lines={logLines}
        droppedCount={dropped}
        tasks={taskList.map((t) => ({ id: t.task_id, name: t.name || t.source }))}
      />

      {/* 对话框 */}
      {editorTask !== undefined && (
        <TaskEditor
          initial={editorTask}
          defaultOptions={defaultOpts}
          onClose={() => setEditorTask(undefined)}
          onSaved={(task, options) => {
            if (editorTask === null) {
              api.addTask(task).then(() => toast("success", `任务「${task.name}」已保存`));
              if (JSON.stringify(options) !== JSON.stringify(defaultOpts)) {
                setDefaultOpts(options);
              }
            } else {
              api.updateTask(task).then((ok) => {
                if (ok) toast("success", `任务「${task.name}」已保存`);
              });
            }
          }}
        />
      )}
      {previewTask && (
        <CommandPreview
          task={previewTask}
          onClose={() => setPreviewTask(null)}
        />
      )}
      {logTask && (
        <LogViewer
          task={logTask}
          onClose={() => setLogTask(null)}
        />
      )}
      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        onClearFinished={() => {
          askConfirm({
            title: "清空所有已完成任务？",
            body: "只把已完成的任务从列表移除，日志与已复制的文件不受影响。",
            okText: "清空",
            danger: true,
            onOk: () => {
              api.clearFinished().then((n) => toast("info", `已清空 ${n} 个已完成任务`));
            },
          });
        }}
      />
      {confirm && (
        <ConfirmDialog
          open
          title={confirm.title}
          body={confirm.body}
          okText={confirm.okText}
          danger={confirm.danger}
          info={confirm.info}
          onCancel={() => setConfirm(null)}
          onOk={() => {
            confirm.onOk();
            setConfirm(null);
          }}
        />
      )}
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <AppInner />
    </ToastProvider>
  );
}
