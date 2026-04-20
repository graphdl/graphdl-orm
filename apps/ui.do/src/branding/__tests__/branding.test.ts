import { describe, expect, it } from 'vitest'
import { FALLBACK_BRANDING, getBranding, DEFAULT_BRANDINGS } from '../branding'

describe('getBranding', () => {
  it('returns the auto.dev UI branding for ui.auto.dev', () => {
    const b = getBranding('ui.auto.dev')
    expect(b.app).toBe('ui.do')
    expect(b.name).toBe('auto.dev')
    expect(b.theme).toBe('light')
    expect(b.nounScope).toBeUndefined()
  })

  it('returns the support branding with a noun-scope filter for support.auto.dev', () => {
    const b = getBranding('support.auto.dev')
    expect(b.app).toBe('support.do')
    expect(b.nounScope).toBeDefined()
    // Support-domain nouns match
    expect(b.nounScope!('Support Request')).toBe(true)
    expect(b.nounScope!('Case')).toBe(true)
    expect(b.nounScope!('Ticket')).toBe(true)
    expect(b.nounScope!('Agent')).toBe(true)
    // Non-support nouns are hidden
    expect(b.nounScope!('Organization')).toBe(false)
    expect(b.nounScope!('Order')).toBe(false)
  })

  it('lowercases and strips ports before lookup (dev 3000, http/https parity)', () => {
    expect(getBranding('UI.AUTO.DEV').app).toBe('ui.do')
    expect(getBranding('support.auto.dev:8080').app).toBe('support.do')
  })

  it('falls back to FALLBACK_BRANDING on unknown hosts', () => {
    expect(getBranding('localhost')).toEqual(FALLBACK_BRANDING)
    expect(getBranding('')).toEqual(FALLBACK_BRANDING)
    expect(getBranding('example.com')).toEqual(FALLBACK_BRANDING)
  })

  it('accepts a custom brandings map', () => {
    const custom = {
      'preview.auto.dev': { app: 'preview.do', name: 'preview', theme: 'dark' as const },
    }
    const b = getBranding('preview.auto.dev', custom)
    expect(b.app).toBe('preview.do')
    expect(b.theme).toBe('dark')
  })

  it('DEFAULT_BRANDINGS is frozen', () => {
    expect(Object.isFrozen(DEFAULT_BRANDINGS)).toBe(true)
  })
})
