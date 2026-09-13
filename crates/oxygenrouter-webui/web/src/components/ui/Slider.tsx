import React, { useCallback, useRef, useState } from "react";

export interface SliderProps {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  disabled?: boolean;
  label?: string;
  showValue?: boolean;
  formatValue?: (value: number) => string;
  id?: string;
}

/**
 * Slider — custom slider, no native range input. role="slider" on the thumb
 * with aria-valuemin/max/now; Arrow keys step, PageUp/Down big-step,
 * Home/End to bounds, click/drag on track jumps.
 */
export function Slider({
  value,
  onChange,
  min = 0,
  max = 100,
  step = 1,
  disabled,
  label,
  showValue = true,
  formatValue,
  id,
}: SliderProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  const pct = max > min ? ((value - min) / (max - min)) * 100 : 0;
  const display = formatValue ? formatValue(value) : String(value);

  const clamp = (n: number) => {
    const stepped = Math.round((n - min) / step) * step;
    return Math.min(max, Math.max(min, Number(stepped.toFixed(6))));
  };

  const valueFromClientX = useCallback(
    (clientX: number) => {
      const el = trackRef.current;
      if (!el) return value;
      const rect = el.getBoundingClientRect();
      const ratio = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
      return clamp(min + ratio * (max - min));
    },
    [min, max, step, value],
  );

  const onPointerDown = (e: React.PointerEvent) => {
    if (disabled) return;
    e.preventDefault();
    setDragging(true);
    const next = valueFromClientX(e.clientX);
    if (next !== value) onChange(next);
    const move = (ev: PointerEvent) => {
      const v = valueFromClientX(ev.clientX);
      if (v !== value) onChange(v);
    };
    const up = () => {
      setDragging(false);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const bigStep = Math.max(step, (max - min) / 10);

  return (
    <div className={`ui-slider-field${disabled ? " is-disabled" : ""}`}>
      {(label || showValue) && (
        <div className="ui-slider-head">
          {label && <span className="ui-slider-label">{label}</span>}
          {showValue && <span className="ui-slider-value font-mono">{display}</span>}
        </div>
      )}
      <div
        ref={trackRef}
        className={`ui-slider-track${dragging ? " is-dragging" : ""}`}
        onPointerDown={onPointerDown}
      >
        <div className="ui-slider-rail" />
        <div className="ui-slider-fill" style={{ width: `${pct}%` }} />
        <div
          id={id}
          role="slider"
          tabIndex={disabled ? -1 : 0}
          aria-label={label}
          aria-valuemin={min}
          aria-valuemax={max}
          aria-valuenow={value}
          aria-disabled={disabled}
          className="ui-slider-thumb"
          style={{ left: `${pct}%` }}
          onKeyDown={(e) => {
            if (disabled) return;
            let next: number | null = null;
            if (e.key === "ArrowRight" || e.key === "ArrowUp") next = clamp(value + step);
            else if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = clamp(value - step);
            else if (e.key === "PageUp") next = clamp(value + bigStep);
            else if (e.key === "PageDown") next = clamp(value - bigStep);
            else if (e.key === "Home") next = min;
            else if (e.key === "End") next = max;
            if (next !== null) { e.preventDefault(); if (next !== value) onChange(next); }
          }}
        />
      </div>
    </div>
  );
}

export default Slider;
