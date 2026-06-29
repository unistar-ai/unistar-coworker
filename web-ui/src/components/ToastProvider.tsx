import { createContext, useCallback, useContext, useState, type ReactNode } from "react";
import * as Toast from "@radix-ui/react-toast";
import { AlertCircle, CheckCircle2, X } from "lucide-react";

type ToastKind = "error" | "success";

interface ToastItem {
  id: number;
  kind: ToastKind;
  message: string;
}

interface ToastContextValue {
  error: (msg: string) => void;
  success: (msg: string) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    // Fallback: no-op if provider isn't mounted (e.g. in tests).
    return { error: () => {}, success: () => {} };
  }
  return ctx;
}

let nextId = 1;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const remove = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const push = useCallback(
    (kind: ToastKind, message: string) => {
      const id = nextId++;
      setToasts((prev) => [...prev, { id, kind, message }]);
    },
    [],
  );

  const ctx: ToastContextValue = {
    error: (msg) => push("error", msg),
    success: (msg) => push("success", msg),
  };

  return (
    <Toast.Provider swipeDirection="right" duration={4000}>
      <ToastContext.Provider value={ctx}>{children}</ToastContext.Provider>
      {toasts.map((t) => (
        <Toast.Root
          key={t.id}
          className={`toast toast-${t.kind}`}
          onOpenChange={(open) => {
            if (!open) remove(t.id);
          }}
        >
          <div className="toast-icon">
            {t.kind === "error" ? (
              <AlertCircle size={16} aria-hidden="true" />
            ) : (
              <CheckCircle2 size={16} aria-hidden="true" />
            )}
          </div>
          <Toast.Title className="toast-title">{t.message}</Toast.Title>
          <Toast.Close className="toast-close" aria-label="Dismiss">
            <X size={14} aria-hidden="true" />
          </Toast.Close>
        </Toast.Root>
      ))}
      <Toast.Viewport className="toast-viewport" />
    </Toast.Provider>
  );
}
