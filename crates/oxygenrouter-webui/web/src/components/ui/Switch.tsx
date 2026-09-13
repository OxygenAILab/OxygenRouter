import React, { useId } from "react";

export interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label?: string;
  description?: string;
  size?: "sm" | "md";
  id?: string;
  name?: string;
}

/**
 * Switch — custom switch, no native checkbox. role="switch" on a button.
 * Space/Enter toggles; arrow keys move focus between grouped switches is left
 * to the browser (they are separate tab stops, like OxygenClaw).
 */
export function Switch({ checked, onChange, disabled, label, description, size = "md", id, name }: SwitchProps) {
  const autoId = useId();
  const switchId = id ?? autoId;

  const track = (
    <button
      type="button"
      role="switch"
      id={switchId}
      name={name}
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      data-value={checked ? "on" : "off"}
      className={`ui-switch ui-switch-${size}${checked ? " is-on" : ""}${disabled ? " is-disabled" : ""}`}
      onClick={() => onChange(!checked)}
      onKeyDown={(e) => {
        if (e.key === "Enter") { e.preventDefault(); onChange(!checked); }
      }}
    >
      <span className="ui-switch-thumb" />
    </button>
  );

  if (!label && !description) return track;
  return (
    <div className={`ui-switch-row${disabled ? " is-disabled" : ""}`}>
      {track}
      {(label || description) && (
        <div className="ui-switch-copy">
          {label && <label className="ui-switch-label" htmlFor={switchId}>{label}</label>}
          {description && <span className="ui-switch-desc">{description}</span>}
        </div>
      )}
    </div>
  );
}

export default Switch;
