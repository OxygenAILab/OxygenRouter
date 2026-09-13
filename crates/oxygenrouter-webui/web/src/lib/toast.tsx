import React from "react";
import { CheckCircle2, AlertCircle, Info, X, AlertTriangle } from "lucide-react";

export type ToastType = "success" | "error" | "info" | "warning";

export interface Toast {
  id: string;
  type: ToastType;
  title: string;
  description?: string;
  duration?: number;
  action?: { label: string; onClick: () => void };
}

type ToastContextType = {
  toasts: Toast[];
  toast: (t: Omit<Toast, "id">) => string;
  dismiss: (id: string) => void;
  success: (title: string, description?: string) => string;
  error: (title: string, description?: string) => string;
  info: (title: string, description?: string) => string;
  warning: (title: string, description?: string) => string;
};

const ToastContext = React.createContext<ToastContextType | null>(null);

export function useToast() {
  const ctx = React.useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

function ToastIcon({ type }: { type: ToastType }) {
  const iconProps = { size: 18, strokeWidth: 2 };
  switch (type) {
    case "success":
      return <CheckCircle2 {...iconProps} />;
    case "error":
      return <AlertCircle {...iconProps} />;
    case "warning":
      return <AlertTriangle {...iconProps} />;
    default:
      return <Info {...iconProps} />;
  }
}

function ToastItem({ toast, onDismiss }: { toast: Toast; onDismiss: () => void }) {
  React.useEffect(() => {
    if (toast.duration !== 0) {
      const timer = setTimeout(onDismiss, toast.duration ?? 4000);
      return () => clearTimeout(timer);
    }
  }, [toast.duration, onDismiss]);

  return (
    <div className={`toast toast-${toast.type}`} role="alert" aria-live="polite">
      <div className="toast-icon">
        <ToastIcon type={toast.type} />
      </div>
      <div className="toast-body">
        <div className="toast-title">{toast.title}</div>
        {toast.description && <div className="toast-description">{toast.description}</div>}
        {toast.action && (
          <button
            className="toast-action"
            onClick={() => {
              toast.action!.onClick();
              onDismiss();
            }}
          >
            {toast.action.label}
          </button>
        )}
      </div>
      <button className="toast-close" onClick={onDismiss} aria-label="Dismiss">
        <X size={14} />
      </button>
    </div>
  );
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = React.useState<Toast[]>([]);

  const dismiss = React.useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const toast = React.useCallback(
    (t: Omit<Toast, "id">) => {
      const id = `toast-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      setToasts((prev) => [...prev, { ...t, id }]);
      return id;
    },
    [],
  );

  const success = React.useCallback(
    (title: string, description?: string) => toast({ type: "success", title, description }),
    [toast],
  );
  const error = React.useCallback(
    (title: string, description?: string) => toast({ type: "error", title, description, duration: 6000 }),
    [toast],
  );
  const info = React.useCallback(
    (title: string, description?: string) => toast({ type: "info", title, description }),
    [toast],
  );
  const warning = React.useCallback(
    (title: string, description?: string) => toast({ type: "warning", title, description, duration: 5000 }),
    [toast],
  );

  return (
    <ToastContext.Provider value={{ toasts, toast, dismiss, success, error, info, warning }}>
      {children}
      <div className="toast-stack" aria-live="polite" aria-relevant="additions">
        {toasts.map((t) => (
          <ToastItem key={t.id} toast={t} onDismiss={() => dismiss(t.id)} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}
