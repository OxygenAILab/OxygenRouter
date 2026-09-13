/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        primary: 'var(--md-primary)',
        'on-primary': 'var(--md-on-primary)',
        secondary: 'var(--md-secondary)',
        tertiary: 'var(--md-tertiary)',
        error: 'var(--md-error)',
        warning: 'var(--md-warning)',
        success: 'var(--md-success)',
        background: 'var(--md-background)',
        surface: 'var(--md-surface)',
        'surface-variant': 'var(--md-surface-variant)',
        'on-surface': 'var(--md-on-surface)',
        'on-surface-variant': 'var(--md-on-surface-variant)',
        outline: 'var(--md-outline)',
        'outline-variant': 'var(--md-outline-variant)',
      },
      fontFamily: {
        sans: ['Inter', '-apple-system', 'BlinkMacSystemFont', 'SF Pro Display', 'Segoe UI', 'sans-serif'],
        mono: ['SF Mono', 'JetBrains Mono', 'Fira Code', 'monospace'],
      },
      borderRadius: {
        sm: '6px',
        md: '8px',
        lg: '12px',
        xl: '16px',
        '2xl': '24px',
      },
      transitionTimingFunction: {
        'ease-apple': 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
        'ease-expo': 'cubic-bezier(0.16, 1, 0.3, 1)',
        'spring': 'cubic-bezier(0.34, 1.56, 0.64, 1)',
      },
    },
  },
  plugins: [],
};
