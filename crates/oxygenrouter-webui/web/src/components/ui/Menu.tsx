import React, { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { LucideIcon } from "lucide-react";

export interface MenuItem {
  id: string;
  label: string;
  icon?: LucideIcon;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

export interface MenuProps {
  trigger: (props: { open: boolean; toggle: () => void }) => React.ReactNode;
  items: MenuItem[];
  align?: "start" | "end";
  header?: React.ReactNode;
  ariaLabel?: string;
}

/**
 * Menu — custom dropdown menu on a portal. Keyboard: Enter/Space/ArrowDown
 * opens, arrows navigate, Enter selects, Esc closes. Click-away closes.
 */
export function Menu({ trigger, items, align = "end", header, ariaLabel }: MenuProps) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const anchorRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<React.CSSProperties>({});
  const listId = useId();

  const update = () => {
    const el = anchorRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setStyle({
      position: "fixed",
      ...(align === "end" ? { right: window.innerWidth - r.right } : { left: r.left }),
      top: r.bottom + 4,
    });
  };

  useEffect(() => {
    if (!open) return;
    update();
    setActive(-1);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); setOpen(false); }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const el = listRef.current?.children[active] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  const toggle = () => setOpen((v) => !v);

  return (
    <div ref={anchorRef} className="ui-menu-anchor">
      {trigger({ open, toggle })}
      {open && createPortal(
        <>
          <div className="ui-popover-backdrop" onMouseDown={() => setOpen(false)} />
          <div
            ref={listRef}
            role="menu"
            aria-label={ariaLabel}
            tabIndex={-1}
            className="ui-menu"
            style={style}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") { e.preventDefault(); setActive((i) => Math.min(i + 1, items.length - 1)); }
              else if (e.key === "ArrowUp") { e.preventDefault(); setActive((i) => Math.max(i - 1, 0)); }
              else if (e.key === "Enter" && active >= 0) {
                e.preventDefault();
                const item = items[active];
                if (item && !item.disabled) { item.onSelect(); setOpen(false); }
              }
            }}
          >
            {header && <div className="ui-menu-header">{header}</div>}
            {items.map((item, i) => {
              const Icon = item.icon;
              return (
                <button
                  key={item.id}
                  role="menuitem"
                  disabled={item.disabled}
                  className={`ui-menu-item${i === active ? " is-active" : ""}${item.danger ? " is-danger" : ""}`}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => { if (!item.disabled) { item.onSelect(); setOpen(false); } }}
                >
                  {Icon && <Icon size={14} />}
                  <span>{item.label}</span>
                </button>
              );
            })}
          </div>
        </>,
        document.body,
      )}
    </div>
  );
}

export default Menu;
