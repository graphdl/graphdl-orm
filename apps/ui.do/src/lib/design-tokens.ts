/**
 * Type-safe TypeScript constants mirroring the AREST design system
 * tokens declared in readings/ui/design.md. These are for callers
 * that need raw values (e.g. inline style, animation libraries,
 * canvas / SVG renderers) rather than Tailwind utility classes.
 *
 * Tailwind classes remain the preferred path for normal markup —
 * they pick up theme switching for free via the CSS variables in
 * src/styles/globals.css. Reach for these constants only when
 * Tailwind's class-based pipeline can't reach.
 */

/* -------------------------------------------------------------------------- */
/* Spacing                                                                    */
/* -------------------------------------------------------------------------- */

/** Spacing step names — match Tailwind utility name & SpacingToken.Name. */
export type SpacingStep = 'xs' | 'sm' | 'md' | 'lg' | 'xl' | '2xl' | '3xl'

/**
 * SpacingToken (4px grid) — identical to the Tailwind theme.extend.spacing
 * entries. Exported in pixels (numbers) so calculations stay first-class.
 */
export const SPACING: Readonly<Record<SpacingStep, number>> = Object.freeze({
  xs: 4,
  sm: 8,
  md: 16,
  lg: 24,
  xl: 32,
  '2xl': 48,
  '3xl': 64,
})

/* -------------------------------------------------------------------------- */
/* Motion                                                                     */
/* -------------------------------------------------------------------------- */

/** Motion role names — identical to MotionToken.Name. */
export type MotionRole = 'fast' | 'normal' | 'enter' | 'exit' | 'emphasized'

export interface MotionTokenValue {
  /** Duration in milliseconds. */
  durationMs: number
  /** CSS easing function. */
  easing: string
}

/**
 * MotionToken values — verbatim from the FORML 2 instance facts. The
 * two durations (150 / 250 ms) are the only allowed motion durations
 * in the design system.
 */
export const MOTION: Readonly<Record<MotionRole, MotionTokenValue>> = Object.freeze({
  fast: { durationMs: 150, easing: 'cubic-bezier(0.4, 0.0, 0.2, 1)' },
  normal: { durationMs: 250, easing: 'cubic-bezier(0.4, 0.0, 0.2, 1)' },
  enter: { durationMs: 250, easing: 'cubic-bezier(0.0, 0.0, 0.2, 1)' },
  exit: { durationMs: 150, easing: 'cubic-bezier(0.4, 0.0, 1, 1)' },
  emphasized: { durationMs: 250, easing: 'cubic-bezier(0.2, 0.0, 0, 1)' },
})

/* -------------------------------------------------------------------------- */
/* Typography                                                                 */
/* -------------------------------------------------------------------------- */

/** Type role names — identical to TypographyScale.Name. */
export type TypeRole =
  | 'display'
  | 'h1'
  | 'h2'
  | 'h3'
  | 'body'
  | 'body-sm'
  | 'caption'
  | 'code'
  | 'label'
  | 'button'

export type FontWeight = 300 | 400 | 500 | 600 | 700
export type FontFamilyKind = 'sans' | 'mono'

export interface TypographyToken {
  /** CSS family name; lookup the cascade via fontFamily.sans / .mono in tailwind.config.js. */
  family: FontFamilyKind
  /** Font weight (numeric). */
  weight: FontWeight
  /** Font size in pixels. */
  fontSizePx: number
  /** Line height in pixels (4px grid). */
  lineHeightPx: number
}

/** TypographyScale values — verbatim from the FORML 2 instance facts. */
export const TYPOGRAPHY: Readonly<Record<TypeRole, TypographyToken>> = Object.freeze({
  display: { family: 'sans', weight: 600, fontSizePx: 36, lineHeightPx: 44 },
  h1: { family: 'sans', weight: 600, fontSizePx: 28, lineHeightPx: 36 },
  h2: { family: 'sans', weight: 600, fontSizePx: 22, lineHeightPx: 28 },
  h3: { family: 'sans', weight: 500, fontSizePx: 18, lineHeightPx: 24 },
  body: { family: 'sans', weight: 400, fontSizePx: 14, lineHeightPx: 20 },
  'body-sm': { family: 'sans', weight: 400, fontSizePx: 13, lineHeightPx: 20 },
  caption: { family: 'sans', weight: 400, fontSizePx: 12, lineHeightPx: 16 },
  label: { family: 'sans', weight: 500, fontSizePx: 12, lineHeightPx: 16 },
  button: { family: 'sans', weight: 500, fontSizePx: 14, lineHeightPx: 20 },
  code: { family: 'mono', weight: 400, fontSizePx: 13, lineHeightPx: 20 },
})

/* -------------------------------------------------------------------------- */
/* Theme modes                                                                */
/* -------------------------------------------------------------------------- */

export type ThemeMode = 'dark' | 'light'

/** Default theme per design.md ('dark' is the default Theme). */
export const DEFAULT_THEME_MODE: ThemeMode = 'dark'
