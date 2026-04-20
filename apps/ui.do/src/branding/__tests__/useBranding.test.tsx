import { afterEach, describe, expect, it, vi } from 'vitest'
import { renderHook } from '@testing-library/react'
import { useBranding } from '../useBranding'

describe('useBranding', () => {
  afterEach(() => { vi.unstubAllGlobals() })

  it('reads window.location.hostname and resolves the matching Branding', () => {
    const { result } = renderHook(() => useBranding({ hostname: 'support.auto.dev' }))
    expect(result.current.app).toBe('support.do')
  })

  it('defaults to FALLBACK_BRANDING when no hostname is available (SSR)', () => {
    const { result } = renderHook(() => useBranding({ hostname: '' }))
    expect(result.current.app).toBe('ui.do')
    expect(result.current.name).toBe('ui.do')
  })
})
