/**
 * Tailwind config for ui.do — wires the AREST design system from
 * readings/ui/design.md into Tailwind's theme.extend.
 *
 * Source of truth: 17 ColorTokens × 2 themes (dark default + .light
 * override), 7 SpacingTokens (xs=4 → 3xl=64), 10 TypographyScales on
 * Inter + JetBrains Mono, and the 5 MotionTokens (150ms / 250ms).
 *
 * Colors are referenced via CSS variables defined in
 * src/styles/globals.css using HSL triplets so Tailwind's
 * <alpha-value> plug-in keeps working (e.g. `bg-accent/50`).
 *
 * `darkMode: "class"` so a JS theme switcher (toggling `.light` on
 * <html>) drives the palette, not just `prefers-color-scheme`.
 */
/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        // Neutral 5-shade scale.
        'neutral-50': 'hsl(var(--color-neutral-50) / <alpha-value>)',
        'neutral-100': 'hsl(var(--color-neutral-100) / <alpha-value>)',
        'neutral-200': 'hsl(var(--color-neutral-200) / <alpha-value>)',
        'neutral-700': 'hsl(var(--color-neutral-700) / <alpha-value>)',
        'neutral-900': 'hsl(var(--color-neutral-900) / <alpha-value>)',
        // Surfaces & semantic roles.
        surface: 'hsl(var(--color-surface) / <alpha-value>)',
        primary: 'hsl(var(--color-primary) / <alpha-value>)',
        secondary: 'hsl(var(--color-secondary) / <alpha-value>)',
        'text-primary': 'hsl(var(--color-text-primary) / <alpha-value>)',
        'text-muted': 'hsl(var(--color-text-muted) / <alpha-value>)',
        border: 'hsl(var(--color-border) / <alpha-value>)',
        accent: 'hsl(var(--color-accent) / <alpha-value>)',
        success: 'hsl(var(--color-success) / <alpha-value>)',
        warning: 'hsl(var(--color-warning) / <alpha-value>)',
        danger: 'hsl(var(--color-danger) / <alpha-value>)',
        info: 'hsl(var(--color-info) / <alpha-value>)',
        // Overlay carries its own alpha — exposed as raw color value.
        overlay: 'var(--color-overlay)',
      },
      spacing: {
        // 4px grid — names match SpacingToken.Name.
        xs: '4px',
        sm: '8px',
        md: '16px',
        lg: '24px',
        xl: '32px',
        '2xl': '48px',
        '3xl': '64px',
      },
      fontFamily: {
        sans: [
          'Inter',
          'system-ui',
          '-apple-system',
          'Segoe UI',
          'Roboto',
          'sans-serif',
        ],
        mono: [
          'JetBrains Mono',
          'SF Mono',
          'Menlo',
          'Consolas',
          'monospace',
        ],
      },
      fontSize: {
        // [size, lineHeight] tuples — direct from TypographyScale facts.
        display: ['36px', '44px'],
        h1: ['28px', '36px'],
        h2: ['22px', '28px'],
        h3: ['18px', '24px'],
        body: ['14px', '20px'],
        'body-sm': ['13px', '20px'],
        caption: ['12px', '16px'],
        label: ['12px', '16px'],
        button: ['14px', '20px'],
        code: ['13px', '20px'],
      },
      transitionDuration: {
        // MotionToken 'fast' / 'exit' = 150ms; the rest = 250ms.
        fast: '150ms',
        normal: '250ms',
        enter: '250ms',
        exit: '150ms',
        emphasized: '250ms',
      },
      transitionTimingFunction: {
        // Easing values lifted verbatim from MotionToken instance facts.
        fast: 'cubic-bezier(0.4, 0.0, 0.2, 1)',
        normal: 'cubic-bezier(0.4, 0.0, 0.2, 1)',
        enter: 'cubic-bezier(0.0, 0.0, 0.2, 1)',
        exit: 'cubic-bezier(0.4, 0.0, 1, 1)',
        emphasized: 'cubic-bezier(0.2, 0.0, 0, 1)',
      },
    },
  },
  plugins: [],
}
