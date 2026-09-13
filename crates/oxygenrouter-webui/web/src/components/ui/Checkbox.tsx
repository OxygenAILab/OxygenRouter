import React, { useId } from "react";
import { Check, Minus } from "lucide-react";

export interface CheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  indeterminate?: boolean;
  disabled?: boolean;
  label?: string;
  id?: string;
  name?: string;
}

/**
 * Checkbox — fully custom, no native input. role="checkbox" on a button with
 * aria-checked supporting tri-state (true/false/"mixed").
 */
export function Checkbox({ checked, onChange, indeterminate, disabled, label, id, name }: CheckboxProps) {
  const autoId = useId();
  const boxId = id ?? autoId;
  const state = indeterminate ? "mixed" : checked;

  const box = (
    <button
      type="button"
      role="checkbox"
      id={boxId}
      name={name}
      aria-checked={state}
      aria-label={label}
      disabled={disabled}
      className={`ui-checkbox${checked || indeterminate ? " is-checked" : ""}${disabled ? " is-disabled" : ""}`}
      onClick={() => onChange(!checked)}
      onKeyDown={(e) => {
        if (e.key === "Enter") { e.preventDefault(); onChange(!checked); }
      }}
    >
      {indeterminate ? <Minus size={12} strokeWidth={2.5} /> : checked ? <Check size={12} strokeWidth={2.5} /> : null}
    </button>
  );

  if (!label) return box;
  return (
    <label className={`ui-checkbox-row${disabled ? " is-disabled" : ""}`} htmlFor={boxId}>
      {box}
      <span className="ui-checkbox-label">{label}</span>
    </label>
  );
}

export default Checkbox;
