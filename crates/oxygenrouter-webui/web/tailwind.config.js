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
        sans: ['var(--font-sans)'],
        mono: ['var(--font-mono)'],
        display: ['var(--font-display)'],
      },
      borderRadius: {
        chip: '6px',
        thumb: '12px',
        sheet: '16px',
        card: '18px',
        panel: '22px',
        hero: '26px',
        pill: '999px',
      },
      transitionTimingFunction: {
        'out-quart': 'cubic-bezier(0.25, 1, 0.5, 1)',
        'spring': 'cubic-bezier(0.32, 0.72, 0, 1)',
      },
    },
  },
  plugins: [],
};
