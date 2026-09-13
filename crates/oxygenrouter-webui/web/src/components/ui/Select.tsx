import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
  hint?: string;
}

export interface SelectProps {
  value: string;
  onChange: (value: string) => void;
  options: SelectOption[];
  placeholder?: string;
  disabled?: boolean;
  label?: string;
  error?: string;
  helper?: string;
  size?: "sm" | "md";
  fullWidth?: boolean;
  id?: string;
  ariaLabel?: string;
}

/**
 * Select — fully custom dropdown. No native <select>.
 * Trigger = button, listbox = portaled popup with full keyboard support:
 * Enter/Space open, ArrowUp/Down navigate, Home/End jump, Enter select,
 * Esc close, type-ahead jumps to matching option.
 */
export function Select({
  value,
  onChange,
  options,
  placeholder = "Select…",
  disabled,
  label,
  error,
  helper,
  size = "md",
  fullWidth,
  id,
  ariaLabel,
}: SelectProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [popupStyle, setPopupStyle] = useState<React.CSSProperties>({});
  const listboxId = useId();
  const selected = options.find((o) => o.value === value);

  const updatePosition = useCallback(() => {
    const el = triggerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom;
    const openUp = spaceBelow < 240 && rect.top > 260;
    setPopupStyle({
      position: "fixed",
      left: rect.left,
      width: rect.width,
      ...(openUp
        ? { bottom: window.innerHeight - rect.top + 4 }
        : { top: rect.bottom + 4 }),
    });
  }, []);

  useEffect(() => {
    if (!open) return;
    updatePosition();
    const onScroll = () => updatePosition();
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const currentIndex = options.findIndex((o) => o.value === value);
    setActiveIndex(currentIndex >= 0 ? currentIndex : 0);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, options, value]);

  useEffect(() => {
    if (!open) return;
    const active = listRef.current?.children[activeIndex] as HTMLElement | undefined;
    active?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, open]);

  const typeAhead = useRef({ seq: "", ts: 0 });
  const handleListKey = (e: React.KeyboardEvent) => {
    const findAhead = (from: number, ch: string): number => {
      const n = options.length;
      for (let step = 1; step <= n; step++) {
        const idx = (from + step) % n;
        if (!options[idx].disabled && options[idx].label.toLowerCase().startsWith(ch)) return idx;
      }
      return -1;
    };
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => {
        for (let s = 1; s <= options.length; s++) {
          const idx = (i + s) % options.length;
          if (!options[idx].disabled) return idx;
        }
        return i;
      });
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => {
        for (let s = 1; s <= options.length; s++) {
          const idx = (i - s + options.length) % options.length;
          if (!options[idx].disabled) return idx;
        }
        return i;
      });
    } else if (e.key === "Home") {
      e.preventDefault();
      setActiveIndex(options.findIndex((o) => !o.disabled));
    } else if (e.key === "End") {
      e.preventDefault();
      for (let i = options.length - 1; i >= 0; i--) {
        if (!options[i].disabled) { setActiveIndex(i); break; }
      }
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const opt = options[activeIndex];
      if (opt && !opt.disabled) {
        onChange(opt.value);
        setOpen(false);
        triggerRef.current?.focus();
      }
    } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const now = Date.now();
      const seq = now - typeAhead.current.ts > 500 ? e.key.toLowerCase() : typeAhead.current.seq + e.key.toLowerCase();
      typeAhead.current = { seq, ts: now };
      const from = activeIndex >= 0 ? activeIndex : options.length - 1;
      const idx = findAhead(from, seq);
      if (idx >= 0) setActiveIndex(idx);
    }
  };

  const select = (opt: SelectOption) => {
    if (opt.disabled) return;
    onChange(opt.value);
    setOpen(false);
    triggerRef.current?.focus();
  };

  const body = (
    <>
      <button
        ref={triggerRef}
        type="button"
        id={id}
        role="combobox"
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-controls={listboxId}
        aria-label={ariaLabel ?? label}
        aria-invalid={!!error}
        disabled={disabled}
        className={`ui-select-trigger ui-select-${size}${error ? " has-error" : ""}${fullWidth ? " is-full" : ""}`}
        onClick={() => { if (!disabled) { setOpen((v) => { if (!v) updatePosition(); return !v; }); } }}
        onKeyDown={(e) => {
          if (!open && (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ")) {
            e.preventDefault();
            updatePosition();
            setOpen(true);
          }
        }}
      >
        <span className={`ui-select-value${selected ? "" : " is-placeholder"}`}>
          {selected?.label ?? placeholder}
        </span>
        <ChevronDown size={15} className={`ui-select-chevron${open ? " is-open" : ""}`} />
      </button>
      {(error || helper) && <p className={`ui-field-msg${error ? " is-error" : ""}`}>{error ?? helper}</p>}
      {open && createPortal(
        <>
          <div className="ui-popover-backdrop" onMouseDown={() => setOpen(false)} />
          <div
            ref={listRef}
            id={listboxId}
            role="listbox"
            aria-label={ariaLabel ?? label}
            tabIndex={-1}
            className="ui-select-list"
            style={popupStyle}
            onKeyDown={handleListKey}
          >
            {options.length === 0 && <div className="ui-select-empty">No options</div>}
            {options.map((opt, i) => {
              const isSelected = opt.value === value;
              return (
                <div
                  key={opt.value}
                  role="option"
                  aria-selected={isSelected}
                  aria-disabled={opt.disabled}
                  className={`ui-select-option${i === activeIndex ? " is-active" : ""}${isSelected ? " is-selected" : ""}${opt.disabled ? " is-disabled" : ""}`}
                  onMouseEnter={() => setActiveIndex(i)}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => select(opt)}
                >
                  <span className="ui-select-option-label">{opt.label}</span>
                  {opt.hint && <span className="ui-select-option-hint">{opt.hint}</span>}
                  {isSelected && <Check size={14} className="ui-select-check" />}
                </div>
              );
            })}
          </div>
        </>,
        document.body,
      )}
    </>
  );

  if (!label) return body;
  return (
    <div className={`ui-field${fullWidth ? " is-full" : ""}`}>
      <label className="ui-field-label" htmlFor={id}>{label}</label>
      {body}
    </div>
  );
}

export default Select;
