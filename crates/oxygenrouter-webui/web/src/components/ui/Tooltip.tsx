import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export interface TooltipProps {
  content: string;
  children: React.ReactNode;
  placement?: "top" | "bottom" | "left" | "right";
  delay?: number;
  disabled?: boolean;
}

/**
 * Tooltip — portal-based, delayed, pointer/focus triggered.
 * Keeps glyphs legible on icon-only controls.
 */
export function Tooltip({ content, children, placement = "top", delay = 300, disabled }: TooltipProps) {
  const [show, setShow] = useState(false);
  const [style, setStyle] = useState<React.CSSProperties>({});
  const anchorRef = useRef<HTMLSpanElement>(null);
  const timer = useRef<number>(0);

  const update = () => {
    const el = anchorRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const base: React.CSSProperties = { position: "fixed" };
    if (placement === "top") { base.left = r.left + r.width / 2; base.top = r.top - 6; base.transform = "translate(-50%, -100%)"; }
    else if (placement === "bottom") { base.left = r.left + r.width / 2; base.top = r.bottom + 6; base.transform = "translate(-50%, 0)"; }
    else if (placement === "left") { base.left = r.left - 6; base.top = r.top + r.height / 2; base.transform = "translate(-100%, -50%)"; }
    else { base.left = r.right + 6; base.top = r.top + r.height / 2; base.transform = "translate(0, -50%)"; }
    setStyle(base);
  };

  const open = () => {
    if (disabled) return;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => { update(); setShow(true); }, delay);
  };
  const close = () => {
    window.clearTimeout(timer.current);
    setShow(false);
  };

  useEffect(() => () => window.clearTimeout(timer.current), []);

  return (
    <span
      ref={anchorRef}
      className="ui-tooltip-anchor"
      onMouseEnter={open}
      onMouseLeave={close}
      onFocus={open}
      onBlur={close}
      onMouseDown={close}
    >
      {children}
      {show && createPortal(
        <span role="tooltip" className={`ui-tooltip is-${placement}`} style={style}>
          {content}
        </span>,
        document.body,
      )}
    </span>
  );
}

export default Tooltip;
