/**
 * Resolve which arest-cli binary the MCP server should shell out to.
 *
 * #841: prefer whichever of target/debug or target/release was built
 * most recently. The recipe in apps/arest's Build Tool fact uses
 * `cargo build --bin arest-cli --features local` which produces
 * target/debug/arest-cli.exe; the MCP server historically defaulted
 * to target/release. Mismatch caused queries to read stale binaries
 * after agents followed the documented recipe. Newer-mtime wins.
 *
 * Falls back to debug if only debug exists, release if only release
 * exists, debug-path if neither (existing-binary check happens at
 * spawn time so the error message is still actionable).
 */

import { existsSync, statSync } from 'fs'
import { resolve } from 'path'

export function resolveArestCli(
  repoRoot: string,
  platform: NodeJS.Platform = process.platform,
): string {
  const exe = platform === 'win32' ? 'arest-cli.exe' : 'arest-cli'
  const debugPath = resolve(repoRoot, 'crates', 'arest', 'target', 'debug', exe)
  const releasePath = resolve(repoRoot, 'crates', 'arest', 'target', 'release', exe)
  const debugMtime = existsSync(debugPath) ? statSync(debugPath).mtimeMs : -Infinity
  const releaseMtime = existsSync(releasePath) ? statSync(releasePath).mtimeMs : -Infinity
  if (debugMtime === -Infinity && releaseMtime === -Infinity) return debugPath
  return debugMtime >= releaseMtime ? debugPath : releasePath
}
