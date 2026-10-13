/** 任务编辑器：路径输入/浏览/盘符切换、实时校验、源估算、空间检查、/MIR 二次确认。 */

import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { OptionsPanel } from "../components/OptionsPanel";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Dialog } from "../components/ui/Dialog";
import { useToast } from "../components/ui/Toast";
import { humanBytes } from "../theme";
import {
  RELATION_MESSAGES,
  defaultOptions,
  makeEmptyTask,
  type CopyOptions,
  type DriveInfo,
  type PathRelation,
  type TaskSnapshot,
} from "../types";

const FIELD = "h-[30px] w-full rounded-md border border-[#CBD5E1] bg-surface px-2.5 text-[13px] outline-none transition-colors focus:border-[#2563EB]";

export interface TaskEditorProps {
  /** null = 新建；非 null = 编辑既有任务 */
  initial: TaskSnapshot | null;
  defaultOptions: CopyOptions;
  onClose: () => void;
  onSaved: (task: TaskSnapshot, options: CopyOptions) => void;
}

export function TaskEditor({ initial, defaultOptions, onClose, onSaved }: TaskEditorProps) {
  const toast = useToast();
  const [source, setSource] = useState(initial?.source ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [dest, setDest] = useState(initial?.dest ?? "");
  const [options, setOptions] = useState<CopyOptions>(
    initial?.options ?? defaultOptions,
  );
  const [relation, setRelation] = useState<PathRelation | null>(null);
  const [estimate, setEstimate] = useState<{ files: number; dirs: number; total_bytes: number } | null>(null);
  const [diskHint, setDiskHint] = useState<{ ok: boolean; text: string } | null>(null);
  const [drives, setDrives] = useState<DriveInfo[]>([]);
  const [command, setCommand] = useState("");
  const [mirConfirm, setMirConfirm] = useState(false);

  const checkTimer = useRef(0);
  const estTimer = useRef(0);
  const cmdTimer = useRef(0);

  const draft = useMemo(
    () => makeEmptyTask({ source, dest, name, options }),
    [source, dest, name, options],
  );

  useEffect(() => {
    api.listDrives().then(setDrives).catch((e) => console.error(e));
  }, []);

  // 路径校验：800ms 防抖（对齐原版输入间隔）
  useEffect(() => {
    if (!source.trim() && !dest.trim()) {
      setRelation(null);
      return;
    }
    window.clearTimeout(checkTimer.current);
    checkTimer.current = window.setTimeout(() => {
      api
        .checkPaths(source, dest)
        .then((rel) => setRelation(rel))
        .catch((e) => console.error(e));
    }, 800);
    return () => window.clearTimeout(checkTimer.current);
  }, [source, dest]);

  // 源目录估算：800ms 防抖自动触发
  useEffect(() => {
    if (!source.trim()) {
      setEstimate(null);
      return;
    }
    window.clearTimeout(estTimer.current);
    estTimer.current = window.setTimeout(() => {
      api
        .scanEstimate(source)
        .then(setEstimate)
        .catch((e) => console.error(e));
    }, 800);
    return () => window.clearTimeout(estTimer.current);
  }, [source]);

  // 命令行预览：300ms 防抖
  useEffect(() => {
    window.clearTimeout(cmdTimer.current);
    if (!source.trim() || !dest.trim()) {
      setCommand("");
      return;
    }
    cmdTimer.current = window.setTimeout(() => {
      api.previewCommand(draft).then(setCommand).catch((e) => console.error(e));
    }, 300);
    return () => window.clearTimeout(cmdTimer.current);
  }, [draft]);

  const browse = (which: "src" | "dst") => {
    api.pickFolder().then((p) => {
      if (!p) return;
      const path = p.replace(/\//g, "\\").replace(/\\+$/, "") + "\\";
      if (which === "src") setSource(path);
      else setDest(path);
    });
  };

  const switchDrive = (root: string) => {
    if (!root) return;
    const tail = source.split("\\").filter(Boolean).pop() ?? "";
    setDest(`${root}${tail}`);
  };

  const doSave = () => {
    onSaved(draft, options);
    onClose();
  };

  const trySave = () => {
    if (!relation || relation !== "ok") {
      const rel = relation ?? "empty";
      toast("error", `任务保存失败：${RELATION_MESSAGES[rel] || "请填写源目录与目标目录"}`);
      return;
    }
    if (options.mirror && initial === null) {
      setMirConfirm(true);
      return;
    }
    doSave();
  };

  const relationOk = relation === "ok";

  return (
    <>
      <Dialog open onClose={onClose} width={760}>
        <div className="flex max-h-[78vh] flex-col overflow-y-auto p-5">
          <h2 className="mb-4 text-sm font-semibold text-foreground">编辑复制任务</h2>

          {/* 源目录 */}
          <div className="flex flex-col gap-3">
            <div className="flex items-center gap-2.5">
              <label className="w-[64px] shrink-0 text-[13px] text-[#64748B]">源目录</label>
              <input
                className={FIELD}
                value={source}
                onChange={(e) => setSource(e.target.value)}
                placeholder="D:\\照片\\2024"
                spellCheck={false}
              />
              <button
                type="button"
                onClick={() => browse("src")}
                className="h-[30px] shrink-0 rounded-md border border-border bg-surface px-3 text-xs text-foreground transition-colors hover:bg-surfaceAlt"
              >
                浏览…
              </button>
            </div>

            <div className="flex items-center gap-2.5">
              <label className="w-[64px] shrink-0 text-[13px] text-[#64748B]">任务名称</label>
              <input
                className={FIELD}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="可选；留空时自动使用源目录名"
                spellCheck={false}
              />
            </div>

            <div className="flex items-center gap-2.5">
              <label className="w-[64px] shrink-0 text-[13px] text-[#64748B]">目标目录</label>
              <input
                className={FIELD}
                value={dest}
                onChange={(e) => setDest(e.target.value)}
                placeholder="E:\\备份\\2024"
                spellCheck={false}
              />
              <button
                type="button"
                onClick={() => browse("dst")}
                className="h-[30px] shrink-0 rounded-md border border-border bg-surface px-3 text-xs text-foreground transition-colors hover:bg-surfaceAlt"
              >
                浏览…
              </button>
              <select
                value=""
                onChange={(e) => switchDrive(e.target.value)}
                className="h-[30px] shrink-0 rounded-md border border-[#CBD5E1] bg-surface px-2 text-xs outline-none"
                title="快速切换目标盘符"
              >
                <option value="">盘符…</option>
                {drives.map((d) => (
                  <option key={d.root} value={d.root}>
                    {d.root}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {/* 实时校验状态行 */}
          <div className="mt-3 min-h-[20px] text-xs">
            {relation === null ? (
              <span className="text-[#94A3B8]">填写路径后自动校验合法性</span>
            ) : relationOk ? (
              <span className="text-[#10B981]">校验通过 ✓</span>
            ) : (
              <span className="text-[#EF4444]">{RELATION_MESSAGES[relation]}</span>
            )}
          </div>

          {/* 源估算 + 目标盘空间 */}
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg bg-surfaceAlt px-3 py-2.5 text-xs">
            {estimate ? (
              <span className="text-[#64748B]">
                源目录：{estimate.files} 个文件 / {estimate.dirs} 个目录，合计{" "}
                <span className="font-mono text-[#2563EB]">
                  {humanBytes(estimate.total_bytes)}
                </span>
              </span>
            ) : (
              <span className="text-[#94A3B8]">
                {source.trim() ? "正在估算源目录大小…" : "填写源目录后自动估算大小"}
              </span>
            )}
            <button
              type="button"
              disabled={!dest.trim()}
              onClick={() => {
                if (!dest.trim()) return;
                api
                  .diskFreeHint(estimate?.total_bytes ?? 0, dest)
                  .then(setDiskHint)
                  .catch((e) => console.error(e));
              }}
              className={`ml-auto h-[26px] shrink-0 rounded-md border px-3 text-xs transition-colors ${
                dest.trim()
                  ? "border-[#93C5FD] bg-[#EFF6FF] text-[#1D4ED8] hover:bg-[#DBEAFE]"
                  : "cursor-not-allowed border-border text-slate-300"
              }`}
            >
              校验目标盘空间
            </button>
          </div>
          {diskHint && (
            <div
              className={`mt-1.5 text-xs ${
                diskHint.ok ? "text-[#10B981]" : "text-[#EF4444]"
              }`}
            >
              {diskHint.text}
            </div>
          )}

          {/* 复制参数 */}
          <div className="mt-4">
            <OptionsPanel value={options} onChange={setOptions} />
          </div>

          {/* 命令行预览 */}
          {command && (
            <pre className="mt-3 break-all whitespace-pre-wrap rounded-lg bg-[#0B1220] p-3 font-mono text-[11px] leading-relaxed text-[#93C5FD]">
              {command}
            </pre>
          )}

          <p className="mt-3 text-[11px] leading-relaxed text-[#94A3B8]">
            提示：本工具直接调用系统 robocopy 完成；勾选「镜像 /MIR」会删除目标端多余文件，请谨慎使用。
          </p>

          <div className="mt-4 flex justify-end gap-2.5">
            <button
              type="button"
              onClick={onClose}
              className="h-9 rounded-md border border-border bg-surface px-4 text-[13px] text-foreground transition-colors hover:bg-surfaceAlt"
            >
              取消
            </button>
            <button
              type="button"
              onClick={trySave}
              className="h-9 rounded-md bg-primary px-6 text-[13px] text-primary-foreground transition-colors hover:bg-[#1D4ED8]"
            >
              保存任务
            </button>
          </div>
        </div>
      </Dialog>

      {/* /MIR 二次确认 */}
      {mirConfirm && (
        <ConfirmDialog
          open
          danger
          title="⚠ 已选择镜像模式 /MIR"
          body={"镜像 /MIR 会把目标端「源里没有、目标里有」的文件与目录全部删除，"
            + "让目标与源完全一致。\n\n请确认：目标目录中不存在需要保留的文件！"}
          okText="确认使用镜像"
          onCancel={() => setMirConfirm(false)}
          onOk={() => {
            setMirConfirm(false);
            doSave();
          }}
        />
      )}
    </>
  );
}
