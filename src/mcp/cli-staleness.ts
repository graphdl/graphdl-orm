/**
 * #842 — detect when AREST_CLI binary is older than the engine
 * source it claims to embody.
 *
 * Pairs with #841 (cli-resolver). #841 picks the right binary
 * between target/debug and target/release; #842 catches the case
 * where neither was rebuilt after a source edit. Together they
 * close the diagnostic loop on the staleness footgun.
 *
 * Returns a human-readable warning when any *.rs file under
 * crates/arest/src/ is newer than the binary (or when the binary
 * is missing). Returns null when the binary is up-to-date.
 */

import { existsSync, readdirSync, statSync } from 'fs'
import { join, basename } from 'path'

export function checkCliStaleness(
  cliPath: string,
  srcDir: string,
): string | null {
  if (!existsSync(cliPath)) {
    return `AREST_CLI binary missing at ${cliPath}; run \`cargo build --bin arest-cli --features local\` to build it.`
  }

  const cliMtime = statSync(cliPath).mtimeMs
  const newest = findNewestRustFile(srcDir, cliMtime)
  if (newest === null) return null

  return `AREST_CLI binary is stale: ${basename(cliPath)} built before ${basename(newest.path)} (newer src by ${formatAge(newest.mtimeMs - cliMtime)}). Rebuild with \`cargo build --bin arest-cli --features local\` so MCP queries reflect engine changes.`
}

interface NewestFile {
  path: string
  mtimeMs: number
}

function findNewestRustFile(dir: string, threshold: number): NewestFile | null {
  if (!existsSync(dir)) return null
  let newest: NewestFile | null = null
  const entries = readdirSync(dir, { withFileTypes: true })
  for (const entry of entries) {
    const full = join(dir, entry.name)
    if (entry.isDirectory()) {
      const nested = findNewestRustFile(full, threshold)
      if (nested && (!newest || nested.mtimeMs > newest.mtimeMs)) newest = nested
    } else if (entry.isFile() && entry.name.endsWith('.rs')) {
      const mtimeMs = statSync(full).mtimeMs
      if (mtimeMs > threshold && (!newest || mtimeMs > newest.mtimeMs)) {
        newest = { path: full, mtimeMs }
      }
    }
  }
  return newest
}

function formatAge(ms: number): string {
  const sec = Math.round(ms / 1000)
  if (sec < 60) return `${sec}s`
  const min = Math.round(sec / 60)
  if (min < 60) return `${min}m`
  const hr = Math.round(min / 60)
  if (hr < 24) return `${hr}h`
  const day = Math.round(hr / 24)
  return `${day}d`
}
