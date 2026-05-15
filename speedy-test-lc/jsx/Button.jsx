import React from "react";
import PropTypes from "prop-types";

const VARIANTS = {
  primary:   "bg-blue-600 text-white hover:bg-blue-700",
  secondary: "bg-gray-100 text-gray-800 hover:bg-gray-200",
  danger:    "bg-red-600 text-white hover:bg-red-700",
  ghost:     "bg-transparent text-gray-700 hover:bg-gray-100",
};

const SIZES = {
  sm: "px-3 py-1.5 text-sm",
  md: "px-4 py-2   text-base",
  lg: "px-6 py-3   text-lg",
};

export function Button({
  children,
  variant = "primary",
  size    = "md",
  loading = false,
  disabled = false,
  onClick,
  type = "button",
  className = "",
  ...rest
}) {
  const base = "inline-flex items-center justify-center rounded-md font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-offset-2 disabled:opacity-50 disabled:cursor-not-allowed";

  return (
    <button
      type={type}
      disabled={disabled || loading}
      onClick={onClick}
      className={`${base} ${VARIANTS[variant]} ${SIZES[size]} ${className}`}
      {...rest}
    >
      {loading && (
        <svg className="mr-2 h-4 w-4 animate-spin" viewBox="0 0 24 24" fill="none">
          <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
          <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8z" />
        </svg>
      )}
      {children}
    </button>
  );
}

Button.propTypes = {
  variant:  PropTypes.oneOf(Object.keys(VARIANTS)),
  size:     PropTypes.oneOf(Object.keys(SIZES)),
  loading:  PropTypes.bool,
  disabled: PropTypes.bool,
  onClick:  PropTypes.func,
  children: PropTypes.node.isRequired,
};
