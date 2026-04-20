/**
 * Hostname-based branding — ui.auto.dev vs support.auto.dev
 * (and anything added downstream) share the same SPA bundle but
 * present different apps. Branding captures everything that should
 * differ between those surfaces:
 *
 *   app         — OpenAPI app scope (drives /api/openapi.json?app=…)
 *   name        — display name for the shell header
 *   theme       — light | dark | system
 *   logo        — optional ReactNode for the sidebar header
 *   nounScope   — optional filter applied to useArestResources so
 *                 support.auto.dev only surfaces support-domain nouns
 *
 * Detection is pure: getBranding(hostname) -> Branding; the React
 * hook layer (useBranding) is a thin wrapper that reads
 * window.location.hostname at render.
 */
import type { ReactNode } from 'react'

export type Theme = 'light' | 'dark' | 'system'

export interface Branding {
  /** OpenAPI app scope — drives useArestResources / useOpenApiSchema. */
  app: string
  /** Display name shown in the shell header. */
  name: string
  /** Theme preference. */
  theme: Theme
  /** Optional logo element. */
  logo?: ReactNode
  /**
   * Optional filter applied to the auto-discovered noun list.
   * Return true to keep the noun in the sidebar, false to hide.
   * Default (no filter) shows every entity schema.
   */
  nounScope?: (nounName: string) => boolean
}

/** Fallback branding — local dev, unknown hosts, tests. */
export const FALLBACK_BRANDING: Branding = {
  app: 'ui.do',
  name: 'ui.do',
  theme: 'system',
}

/**
 * Known hostname → Branding map. The lookup is exact-match by
 * hostname; wildcards / prefixes can be layered on via a custom
 * `brandings` argument to getBranding.
 *
 * These map to the app slugs registered in the worker's scope
 * directory. Any new public host should be added here.
 */
export const DEFAULT_BRANDINGS: Readonly<Record<string, Branding>> = Object.freeze({
  'ui.auto.dev': {
    app: 'ui.do',
    name: 'auto.dev',
    theme: 'light',
  },
  'support.auto.dev': {
    app: 'support.do',
    name: 'auto.dev support',
    theme: 'light',
    // Only surface support-domain nouns. Extend this list as the
    // support schema grows; the brief explicitly calls out the
    // noun-scope filter as part of support-branding.
    nounScope: (noun) =>
      /^(Support\s*Request|Case|Ticket|Message|Article|Agent|Customer|Category|Priority|Status)$/i
        .test(noun),
  },
})

/**
 * Resolve the branding for a hostname. Falls back to `FALLBACK_BRANDING`
 * when no entry matches. Accepts an override map so callers can slot
 * in per-app brandings without mutating the default.
 */
export function getBranding(
  hostname: string,
  brandings: Readonly<Record<string, Branding>> = DEFAULT_BRANDINGS,
): Branding {
  if (!hostname) return FALLBACK_BRANDING
  const normalised = hostname.toLowerCase().replace(/:\d+$/, '')
  return brandings[normalised] ?? FALLBACK_BRANDING
}
