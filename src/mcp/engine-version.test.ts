/**
 * version/staleness instrument — pure comparison logic for the
 * `engine_version` MCP verb.
 *
 * CONTEXT: the MCP pins AREST_CLI at startup and re-spawns that exact
 * path on every call. A startup resolver that picks a stale binary
 * (different build profile than the one being rebuilt) means hours of
 * "deployed + verified live" can silently run a stale engine. The
 * running binary self-reporting its git SHA + build time is the
 * unambiguous fix; this module compares that self-report against the
 * repo's current HEAD so "is the live engine current?" is one call.
 *
 * These tests feed a fake version-JSON (what `arest-cli version`
 * printed) + a fake HEAD SHA and assert the up_to_date / behind shape.
 * Mirrors cli-resolver.test.ts.
 */

import { describe, it, expect } from 'vitest'
import { compareEngineVersion } from './engine-version.js'

describe('compareEngineVersion', () => {
  const HEAD = '53cdc48eb2c2dab9997ef0f37f4be40f17cf0267'

  it('reports up_to_date when live sha equals HEAD', () => {
    const versionJson = JSON.stringify({ sha: HEAD, built: '2026-06-04T12:00:00Z', pkg: '0.9.0' })
    const r = compareEngineVersion(versionJson, HEAD)
    expect(r.live_sha).toBe(HEAD)
    expect(r.live_built).toBe('2026-06-04T12:00:00Z')
    expect(r.head_sha).toBe(HEAD)
    expect(r.up_to_date).toBe(true)
    expect(r.behind_message).toBeNull()
  })

  it('reports NOT up_to_date when live sha differs from HEAD', () => {
    const liveSha = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    const versionJson = JSON.stringify({ sha: liveSha, built: '2026-05-01T00:00:00Z', pkg: '0.9.0' })
    const r = compareEngineVersion(versionJson, HEAD)
    expect(r.live_sha).toBe(liveSha)
    expect(r.head_sha).toBe(HEAD)
    expect(r.up_to_date).toBe(false)
    expect(r.behind_message).toContain(liveSha.slice(0, 12))
    expect(r.behind_message).toContain(HEAD.slice(0, 12))
    // The whole point: the message must tell the operator to relaunch.
    expect(r.behind_message?.toLowerCase()).toContain('rebuild')
  })

  it('compares full SHA, not a truncated prefix (avoids false up_to_date on a 12-char collision)', () => {
    // Same first 12 chars, diverge after — must NOT be up_to_date.
    const liveSha = '53cdc48eb2c2FFFFFFFFFFFFFFFFFFFFFFFFFFFFF'
    const versionJson = JSON.stringify({ sha: liveSha, built: '2026-05-01T00:00:00Z', pkg: '0.9.0' })
    const r = compareEngineVersion(versionJson, HEAD)
    expect(r.up_to_date).toBe(false)
  })

  it('treats a build-time "unknown" sha as never up_to_date and flags it', () => {
    const versionJson = JSON.stringify({ sha: 'unknown', built: '2026-06-04T12:00:00Z', pkg: '0.9.0' })
    const r = compareEngineVersion(versionJson, HEAD)
    expect(r.live_sha).toBe('unknown')
    expect(r.up_to_date).toBe(false)
    expect(r.behind_message).toBeTruthy()
    expect(r.behind_message?.toLowerCase()).toContain('unknown')
  })

  it('treats an unknown HEAD (git unavailable at query time) as indeterminate, not up_to_date', () => {
    const versionJson = JSON.stringify({ sha: HEAD, built: '2026-06-04T12:00:00Z', pkg: '0.9.0' })
    const r = compareEngineVersion(versionJson, 'unknown')
    expect(r.head_sha).toBe('unknown')
    expect(r.up_to_date).toBe(false)
    expect(r.behind_message).toBeTruthy()
  })

  it('handles malformed version JSON without throwing and is not up_to_date', () => {
    const r = compareEngineVersion('not json at all', HEAD)
    expect(r.live_sha).toBe('unknown')
    expect(r.live_built).toBe('unknown')
    expect(r.up_to_date).toBe(false)
    expect(r.behind_message).toBeTruthy()
  })

  it('passes through pkg version when present', () => {
    const versionJson = JSON.stringify({ sha: HEAD, built: '2026-06-04T12:00:00Z', pkg: '0.9.0' })
    const r = compareEngineVersion(versionJson, HEAD)
    expect(r.pkg).toBe('0.9.0')
  })

  it('tolerates surrounding whitespace/newlines on the HEAD value (raw git output)', () => {
    const versionJson = JSON.stringify({ sha: HEAD, built: '2026-06-04T12:00:00Z', pkg: '0.9.0' })
    const r = compareEngineVersion(versionJson, `  ${HEAD}\n`)
    expect(r.head_sha).toBe(HEAD)
    expect(r.up_to_date).toBe(true)
  })
})
