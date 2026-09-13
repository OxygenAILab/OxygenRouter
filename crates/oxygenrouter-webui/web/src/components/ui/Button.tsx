import React from "react";

export type ButtonVariant = "filled" | "tonal" | "outlined" | "ghost" | "text";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  leftIcon?: React.ReactNode;
  rightIcon?: React.ReactNode;
  fullWidth?: boolean;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "filled", size = "md", loading = false, leftIcon, rightIcon, fullWidth = false, className = "", children, disabled, ...props },
  ref,
) {
  return (
    <button
      ref={ref}
      className={`ui-button ui-button-${variant} ui-button-${size}${fullWidth ? " ui-button-full" : ""}${className ? ` ${className}` : ""}`}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...props}
    >
      {loading ? <span className="ui-button-spinner" aria-hidden="true" /> : leftIcon ? <span className="ui-button-icon" aria-hidden="true">{leftIcon}</span> : null}
      <span className="ui-button-label">{children}</span>
      {!loading && rightIcon ? <span className="ui-button-icon" aria-hidden="true">{rightIcon}</span> : null}
    </button>
  );
});

export default Button;
