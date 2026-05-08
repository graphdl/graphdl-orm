/**
 * #841 — resolveArestCli prefers the most-recently-built binary
 * between target/debug and target/release. Eliminates the
 * recipe-vs-default mismatch where MCP queries would silently read
 * a stale binary after agents rebuilt the other profile.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { resolveArestCli } from './cli-resolver.js'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, utimesSync, existsSync } from 'fs'
import { join, resolve } from 'path'
import { tmpdir } from 'os'

describe('#841 resolveArestCli', () => {
  let tempRoot: string

  beforeEach(() => {
    tempRoot = mkdtempSync(join(tmpdir(), 'arest-cli-resolver-'))
    mkdirSync(join(tempRoot, 'crates/arest/target/debug'), { recursive: true })
    mkdirSync(join(tempRoot, 'crates/arest/target/release'), { recursive: true })
  })

  afterEach(() => {
    if (tempRoot && existsSync(tempRoot)) {
      rmSync(tempRoot, { recursive: true, force: true })
    }
  })

  function writeBinary(profile: 'debug' | 'release', mtime: Date): string {
    const path = resolve(tempRoot, 'crates/arest/target', profile, 'arest-cli.exe')
    writeFileSync(path, 'fake binary')
    utimesSync(path, mtime, mtime)
    return path
  }

  it('returns debug path when only debug binary exists', () => {
    const debugPath = writeBinary('debug', new Date())
    const result = resolveArestCli(tempRoot, 'win32')
    expect(result).toBe(debugPath)
  })

  it('returns release path when only release binary exists', () => {
    const releasePath = writeBinary('release', new Date())
    const result = resolveArestCli(tempRoot, 'win32')
    expect(result).toBe(releasePath)
  })

  it('returns debug path when debug is newer than release', () => {
    const olderRelease = new Date('2026-05-01T00:00:00Z')
    const newerDebug = new Date('2026-05-08T00:00:00Z')
    writeBinary('release', olderRelease)
    const debugPath = writeBinary('debug', newerDebug)
    const result = resolveArestCli(tempRoot, 'win32')
    expect(result).toBe(debugPath)
  })

  it('returns release path when release is newer than debug', () => {
    const olderDebug = new Date('2026-05-01T00:00:00Z')
    const newerRelease = new Date('2026-05-08T00:00:00Z')
    writeBinary('debug', olderDebug)
    const releasePath = writeBinary('release', newerRelease)
    const result = resolveArestCli(tempRoot, 'win32')
    expect(result).toBe(releasePath)
  })

  it('returns debug path when neither binary exists (fallback for actionable error message)', () => {
    const result = resolveArestCli(tempRoot, 'win32')
    expect(result).toBe(resolve(tempRoot, 'crates/arest/target/debug/arest-cli.exe'))
  })

  it('uses arest-cli (no .exe) on non-Windows platforms', () => {
    mkdirSync(join(tempRoot, 'crates/arest/target/debug'), { recursive: true })
    const path = resolve(tempRoot, 'crates/arest/target/debug/arest-cli')
    writeFileSync(path, 'fake binary')
    const result = resolveArestCli(tempRoot, 'linux')
    expect(result).toBe(path)
    expect(result.endsWith('.exe')).toBe(false)
  })

  it('on tie (equal mtimes), prefers debug', () => {
    const sameMtime = new Date('2026-05-08T00:00:00Z')
    const debugPath = writeBinary('debug', sameMtime)
    writeBinary('release', sameMtime)
    const result = resolveArestCli(tempRoot, 'win32')
    expect(result).toBe(debugPath)
  })
})
