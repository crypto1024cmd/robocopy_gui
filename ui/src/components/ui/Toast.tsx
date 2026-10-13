/** 应用内 Toast（右上角浮层，2.6s 自动消失，对齐原版 Toast 行为）。 */

import { createContext, useCallback, useContext, useState, type ReactNode } from "react";

export type ToastKind = "info" | "success" | "error" | "warning";

interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
}

const ToastCtx = createContext<(kind: ToastKind, text: string) => void>(() => {});

export function useToast() {
  return useContext(ToastCtx);
}

const KIND_BAR: Record<ToastKind, string> = {
  info: "bg-[#06B6D4]",
  success: "bg-[#10B981]",
  error: "bg-[#EF4444]",
  warning: "bg-[#F59E0B]",
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const push = useCallback((kind: ToastKind, text: string) => {
    const id = Date.now() + Math.random();
    setToasts((prev) => [...prev, { id, kind, text }]);
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 2600);
  }, []);

  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="pointer-events-none fixed right-4 top-[72px] z-[200] flex flex-col gap-2">
        {toasts.map((t) => (
          <div
            key={t.id}
            className="animate-in fade-in slide-in-from-right-4 duration-200 flex w-72 overflow-hidden rounded-lg bg-card shadow-card ring-1 ring-border"
          >
            <div className={`w-1 shrink-0 ${KIND_BAR[t.kind]}`} />
            <div className="px-3 py-2.5 text-xs leading-relaxed text-foreground">
              {t.text}
            </div>
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}
