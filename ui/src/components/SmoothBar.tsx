/** 平滑进度条：目标回退直接跳变、增长用 180ms 补间（对齐原版 SmoothBar）。 */

import { useEffect, useRef, useState } from "react";

export interface SmoothBarProps {
  /** 0..100 */
  value: number;
  height?: number;
  /** 轨道底色（顶栏深色 / 卡片浅色） */
  trackClass?: string;
  className?: string;
}

export function SmoothBar({
  value,
  height = 8,
  trackClass = "bg-[#E2E8F0]",
  className = "",
}: SmoothBarProps) {
  const [display, setDisplay] = useState(value);
  const displayRef = useRef(value);

  useEffect(() => {
    const target = Math.max(0, Math.min(100, value));
    // 目标回退 → 不做补间，直接跳（避免「倒着走」）
    if (target < displayRef.current) {
      displayRef.current = target;
      setDisplay(target);
      return;
    }
    const start = displayRef.current;
    const delta = target - start;
    if (Math.abs(delta) < 0.01) {
      displayRef.current = target;
      setDisplay(target);
      return;
    }
    let raf = 0;
    const t0 = performance.now();
    const tick = (now: number) => {
      const p = Math.min(1, (now - t0) / 180);
      const v = start + delta * p;
      displayRef.current = v;
      setDisplay(v);
      if (p < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [value]);

  return (
    <div
      className={`relative w-full overflow-hidden rounded-full ${trackClass} ${className}`}
      style={{ height }}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(display)}
    >
      <div
        className="absolute inset-y-0 left-0 rounded-full bg-[#2563EB]"
        style={{ width: `${display}%` }}
      >
        <div className="absolute inset-x-0 top-0 h-1/2 rounded-full bg-[#93C5FD]/60" />
      </div>
    </div>
  );
}
