import React, { useId } from "react";

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  helper?: string;
  error?: string;
}

export default function Input({ label, helper, error, id, className = "", ...props }: InputProps) {
  const generatedId = useId();
  const inputId = id ?? `input-${generatedId}`;
  const messageId = `${inputId}-message`;
  return <div className="ui-field">
    {label && <label className="ui-field-label" htmlFor={inputId}>{label}</label>}
    <input {...props} id={inputId} className={`ui-input${error ? " has-error" : ""}${className ? ` ${className}` : ""}`} aria-invalid={error ? true : undefined} aria-describedby={helper || error ? messageId : undefined} />
    {(error || helper) && <div id={messageId} className={error ? "ui-field-error" : "ui-field-helper"}>{error || helper}</div>}
  </div>;
}
