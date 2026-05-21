/**
 * useBranding — React hook that resolves the current Branding from
 * window.location.hostname. Safe in SSR / build-time (no window ⇒
 * FALLBACK_BRANDING).
 */
import { useMemo } from 'react'
import {
  DEFAULT_BRANDINGS,
  FALLBACK_BRANDING,
  getBranding,
  type Branding,
} from './branding'

export interface UseBrandingOptions {
  /** Optional override map. Defaults to DEFAULT_BRANDINGS. */
  brandings?: Readonly<Record<string, Branding>>
  /** Optional hostname override — handy for tests / preview runs. */
  hostname?: string
}

export function useBranding(options: UseBrandingOptions = {}): Branding {
  const brandings = options.brandings ?? DEFAULT_BRANDINGS
  // Local-dev override: VITE_AREST_HOST lets a dev running on
  // localhost:5174 pick up a production hostname's branding (app slug,
  // noun-scope filter) without faking DNS. e.g. set
  // VITE_AREST_HOST=support.auto.dev to render the support app locally.
  // See apps/ui.do/LOCAL-DEV.md. Falls back to the real hostname.
  const envHost =
    (import.meta.env.VITE_AREST_HOST as string | undefined) || undefined
  const hostname = options.hostname
    ?? envHost
    ?? (typeof window !== 'undefined' ? window.location.hostname : '')

  return useMemo(() => {
    if (!hostname) return FALLBACK_BRANDING
    return getBranding(hostname, brandings)
  }, [hostname, brandings])
}
