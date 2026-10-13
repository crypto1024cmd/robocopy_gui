/** 空任务状态（对齐原版 EmptyState：圆形虚线图标 + 主文案 + 副文案 + 按钮）。 */

export function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-4 py-24 text-center">
      <div className="flex h-20 w-20 items-center justify-center rounded-full border-2 border-dashed border-[#CBD5E1]">
        <span className="text-4xl text-[#94A3B8]">📂</span>
      </div>
      <div>
        <div className="text-sm font-semibold text-foreground">还没有复制任务</div>
        <div className="mt-1 text-xs text-[#94A3B8]">
          新建一个「源目录 → 目标目录」任务，然后点击「开始全部」开始高速搬运
        </div>
      </div>
      <button
        type="button"
        onClick={onAdd}
        className="h-9 rounded-md bg-primary px-5 text-[13px] text-primary-foreground transition-colors hover:bg-[#1D4ED8]"
      >
        ＋ 新建任务
      </button>
    </div>
  );
}
