/**
 * #842 — checkCliStaleness compares the AREST_CLI binary's mtime
 * against the newest source file under crates/arest/src/. Returns
 * a warning message when the binary is older, null otherwise.
 *
 * Closes the diagnostic loop on the symptom that bit this session:
 * agent edits engine source, rebuilds the wrong artifact, queries
 * silently return stale results. Pairs with #841 (prefer-newer
 * binary resolution) — together they catch both halves of the
 * staleness footgun.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { checkCliStaleness } from './cli-staleness.js'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, utimesSync, existsSync } from 'fs'
import { join, resolve } from 'path'
import { tmpdir } from 'os'

describe('#842 checkCliStaleness', () => {
  let tempRoot: string
  let srcDir: string
  let cliPath: string

  beforeEach(() => {
    tempRoot = mkdtempSync(join(tmpdir(), 'arest-staleness-'))
    srcDir = join(tempRoot, 'crates/arest/src')
    mkdirSync(srcDir, { recursive: true })
    mkdirSync(join(tempRoot, 'crates/arest/target/debug'), { recursive: true })
    cliPath = resolve(tempRoot, 'crates/arest/target/debug/arest-cli.exe')
  })

  afterEach(() => {
    if (tempRoot && existsSync(tempRoot)) {
      rmSync(tempRoot, { recursive: true, force: true })
    }
  })

  function writeWithMtime(path: string, content: string, mtime: Date) {
    writeFileSync(path, content)
    utimesSync(path, mtime, mtime)
  }

  it('returns null when binary is newer than all source files', () => {
    writeWithMtime(join(srcDir, 'lib.rs'), '// lib', new Date('2026-05-01T00:00:00Z'))
    writeWithMtime(join(srcDir, 'ast.rs'), '// ast', new Date('2026-05-02T00:00:00Z'))
    writeWithMtime(cliPath, 'binary', new Date('2026-05-03T00:00:00Z'))
    const result = checkCliStaleness(cliPath, srcDir)
    expect(result).toBeNull()
  })

  it('returns warning when any source file is newer than the binary', () => {
    writeWithMtime(cliPath, 'binary', new Date('2026-05-01T00:00:00Z'))
    writeWithMtime(join(srcDir, 'lib.rs'), '// lib', new Date('2026-05-08T00:00:00Z'))
    const result = checkCliStaleness(cliPath, srcDir)
    expect(result).not.toBeNull()
    expect(result).toMatch(/stale|older|newer/i)
    expect(result).toContain('lib.rs')
  })

  it('detects newer files in nested subdirectories', () => {
    mkdirSync(join(srcDir, 'cli'), { recursive: true })
    writeWithMtime(cliPath, 'binary', new Date('2026-05-01T00:00:00Z'))
    writeWithMtime(join(srcDir, 'cli', 'entry.rs'), '// nested', new Date('2026-05-08T00:00:00Z'))
    const result = checkCliStaleness(cliPath, srcDir)
    expect(result).not.toBeNull()
    expect(result).toContain('entry.rs')
  })

  it('returns null when no .rs files exist under src', () => {
    writeWithMtime(cliPath, 'binary', new Date('2026-05-01T00:00:00Z'))
    const result = checkCliStaleness(cliPath, srcDir)
    expect(result).toBeNull()
  })

  it('returns warning when binary does not exist', () => {
    writeWithMtime(join(srcDir, 'lib.rs'), '// lib', new Date('2026-05-08T00:00:00Z'))
    const result = checkCliStaleness(cliPath, srcDir)
    expect(result).not.toBeNull()
    expect(result).toMatch(/missing|not found|does not exist/i)
  })

  it('ignores non-.rs files under src (e.g. *.md, target/, etc.)', () => {
    writeWithMtime(cliPath, 'binary', new Date('2026-05-01T00:00:00Z'))
    writeWithMtime(join(srcDir, 'README.md'), '// notes', new Date('2026-05-08T00:00:00Z'))
    const result = checkCliStaleness(cliPath, srcDir)
    expect(result).toBeNull()
  })
})
