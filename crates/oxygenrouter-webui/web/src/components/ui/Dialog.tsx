import React, { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import Button from "./Button";

type DialogSize = "sm" | "md" | "lg" | "xl";
export interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  footer?: React.ReactNode;
  size?: DialogSize;
  children: React.ReactNode;
  closeOnOverlay?: boolean;
  closable?: boolean;
  className?: string;
}

const focusable = "button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex=\"-1\"])";

export default function Dialog({ open, onClose, title, description, footer, size = "md", children, closeOnOverlay = true, closable = true, className = "" }: DialogProps) {
  const titleId = useId();
  const descriptionId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    previousFocus.current = document.activeElement as HTMLElement;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const timer = window.setTimeout(() => {
      const target = dialogRef.current?.querySelector<HTMLElement>("[data-dialog-autofocus], [data-dialog-close], input, select, textarea, button, [tabindex]:not([tabindex=\"-1\"])");
      target?.focus();
    }, 0);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && closable) { event.preventDefault(); onClose(); return; }
      if (event.key !== "Tab" || !dialogRef.current) return;
      const elements = Array.from(dialogRef.current.querySelectorAll<HTMLElement>(focusable));
      if (!elements.length) return;
      const first = elements[0];
      const last = elements[elements.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => { window.clearTimeout(timer); document.body.style.overflow = previousOverflow; document.removeEventListener("keydown", onKeyDown); previousFocus.current?.focus(); };
  }, [open, onClose, closable]);

  if (!open) return null;
  return createPortal(
    <div className="ui-dialog-overlay" onMouseDown={(event) => { if (closeOnOverlay && event.target === event.currentTarget) onClose(); }}>
      <div ref={dialogRef} className={`ui-dialog ui-dialog-${size}${className ? ` ${className}` : ""}`} role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={description ? descriptionId : undefined}>
        <div className="ui-dialog-header">
          <div><h2 id={titleId}>{title}</h2>{description && <p id={descriptionId}>{description}</p>}</div>
          {closable && <Button data-dialog-close variant="ghost" size="sm" aria-label="Close dialog" onClick={onClose} leftIcon={<X size={18} />} />}
        </div>
        <div className="ui-dialog-body">{children}</div>
        {footer && <div className="ui-dialog-footer">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}
