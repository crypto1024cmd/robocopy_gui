/** 复制选项面板（任务编辑器与右栏「新任务默认参数」共用，对齐原版 OptionsPanel）。 */

import { useState } from "react";
import { summaryText } from "../theme";
import type { CopyOptions, HashAlgo, VerifyLevel } from "../types";

export interface OptionsPanelProps {
  value: CopyOptions;
  onChange: (options: CopyOptions) => void;
}

/** 多段切换按钮。 */
function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="inline-flex overflow-hidden rounded-md border border-[#CBD5E1]">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={`h-[26px] border-l px-3 text-xs first:border-l-0 transition-colors ${
            value === o.value
              ? "bg-[#2563EB] text-white"
              : "bg-surface text-foreground hover:bg-surfaceAlt"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

const VERIFY_OPTIONS: { value: VerifyLevel; label: string }[] = [
  { value: "none", label: "不校验" },
  { value: "quick", label: "大小+时间戳" },
  { value: "strict", label: "哈希校验" },
];

const HASH_LABELS: Record<HashAlgo, string> = {
  md5: "MD5（最快）",
  sha1: "SHA1（较安全）",
  sha256: "SHA256（最安全）",
};

export function OptionsPanel({ value, onChange }: OptionsPanelProps) {
  const [showAdvanced, setShowAdvanced] = useState(false);
  const patch = (p: Partial<CopyOptions>) => onChange({ ...value, ...p });

  const hint =
    value.verify_level === "none"
      ? "不进行复制后校验，速度最快"
      : value.verify_level === "quick"
        ? "复制结束后对比每个文件的大小和修改时间（2 秒容差），开销极低"
        : `复制结束后对每个文件计算 ${value.hash_algo.toUpperCase()} 并与源端逐字节比对，最严格但最耗时`;

  return (
    <div className="flex flex-col gap-2.5">
      {/* 复制模式 */}
      <div className="flex items-center gap-3">
        <span className="w-[64px] shrink-0 text-xs text-[#64748B]">复制模式</span>
        <Segmented
          value={value.mirror ? "mir" : "e"}
          onChange={(v) => patch({ mirror: v === "mir" })}
          options={[
            { value: "e", label: "普通 /E" },
            { value: "mir", label: "镜像 /MIR" },
          ]}
        />
      </div>

      {/* 线程数 + 预设 */}
      <div className="flex items-center gap-3">
        <span className="w-[64px] shrink-0 text-xs text-[#64748B]">多线程</span>
        <input
          type="number"
          min={1}
          max={128}
          value={value.threads}
          onChange={(e) => patch({ threads: Math.trunc(Number(e.target.value) || 1) })}
          className="h-[26px] w-16 rounded-md border border-[#CBD5E1] bg-surface px-2 text-xs outline-none focus:border-[#2563EB]"
        />
        {[8, 16, 32, 64].map((n) => (
          <button
            key={n}
            type="button"
            onClick={() => patch({ threads: n })}
            className={`h-[22px] rounded px-2 text-[11px] transition-colors ${
              value.threads === n
                ? "bg-[#DBEAFE] text-[#1D4ED8]"
                : "text-[#64748B] hover:bg-surfaceAlt"
            }`}
          >
            {n}
          </button>
        ))}
      </div>

      {/* 校验级别 */}
      <div className="flex items-center gap-3">
        <span className="w-[64px] shrink-0 text-xs text-[#64748B]">复制后校验</span>
        <Segmented
          value={value.verify_level}
          onChange={(v) => patch({ verify_level: v })}
          options={VERIFY_OPTIONS}
        />
      </div>

      {/* 哈希算法 */}
      <div className="flex items-center gap-3">
        <span className="w-[64px] shrink-0 text-xs text-[#64748B]">哈希算法</span>
        <select
          value={value.hash_algo}
          onChange={(e) => patch({ hash_algo: e.target.value as HashAlgo })}
          disabled={value.verify_level !== "strict"}
          className={`h-[26px] rounded-md border border-[#CBD5E1] bg-surface px-2 text-xs outline-none ${
            value.verify_level !== "strict" ? "cursor-not-allowed opacity-50" : ""
          }`}
        >
          {(Object.keys(HASH_LABELS) as HashAlgo[]).map((k) => (
            <option key={k} value={k}>
              {HASH_LABELS[k]}
            </option>
          ))}
        </select>
      </div>

      {/* 高级选项 */}
      <div className="flex flex-col gap-1.5">
        <button
          type="button"
          onClick={() => setShowAdvanced((s) => !s)}
          className="w-fit text-[11px] text-[#2563EB] transition-colors hover:text-[#1D4ED8]"
        >
          {showAdvanced ? "收起高级选项 ▲" : "高级选项（试运行 / 断点重启）▼"}
        </button>
        {showAdvanced && (
          <div className="flex flex-col gap-1.5 rounded-lg bg-surfaceAlt p-2.5">
            <label className="flex cursor-pointer items-center gap-2 text-xs text-foreground">
              <input
                type="checkbox"
                checked={value.dry_run}
                onChange={(e) => patch({ dry_run: e.target.checked })}
                className="h-3.5 w-3.5 accent-[#2563EB]"
              />
              试运行 /L（只列出将要复制的文件，不写入任何数据）
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-xs text-foreground">
              <input
                type="checkbox"
                checked={value.restartable}
                onChange={(e) => patch({ restartable: e.target.checked })}
                className="h-3.5 w-3.5 accent-[#2563EB]"
              />
              断点重启 /Z（网络不稳建议开启，速度明显下降）
            </label>
          </div>
        )}
      </div>

      {/* 摘要 + 校验说明 */}
      <div className="border-t border-border pt-2 text-[11px] leading-relaxed text-[#94A3B8]">
        <div>{summaryText(value)}</div>
        <div>{hint}</div>
      </div>
    </div>
  );
}
