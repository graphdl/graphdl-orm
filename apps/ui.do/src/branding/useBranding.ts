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
  const hostname = options.hostname
    ?? (typeof window !== 'undefined' ? window.location.hostname : '')

  return useMemo(() => {
    if (!hostname) return FALLBACK_BRANDING
    return getBranding(hostname, brandings)
  }, [hostname, brandings])
}
