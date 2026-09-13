import React, { useRef } from "react";

export interface SegmentedControlProps<T extends string> {
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string; icon?: React.ElementType }[];
  size?: "sm" | "md";
  ariaLabel?: string;
}

/**
 * SegmentedControl — custom segmented control (tablist semantics).
 * Roving tabindex: arrows move focus+selection (OxygenClaw pattern).
 */
export function SegmentedControl<T extends string>({ value, onChange, options, size = "md", ariaLabel }: SegmentedControlProps<T>) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  return (
    <div className={`ui-segmented ui-segmented-${size}`} role="tablist" aria-label={ariaLabel}>
      {options.map((opt, i) => {
        const Icon = opt.icon;
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            ref={(el) => { refs.current[i] = el; }}
            type="button"
            role="tab"
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            className={`ui-segmented-item${active ? " is-active" : ""}`}
            onClick={() => onChange(opt.value)}
            onKeyDown={(e) => {
              let next = -1;
              if (e.key === "ArrowRight") next = (i + 1) % options.length;
              else if (e.key === "ArrowLeft") next = (i - 1 + options.length) % options.length;
              if (next >= 0) { e.preventDefault(); onChange(options[next].value); refs.current[next]?.focus(); }
            }}
          >
            {Icon && <Icon size={13} />}
            <span>{opt.label}</span>
          </button>
        );
      })}
    </div>
  );
}

export default SegmentedControl;
