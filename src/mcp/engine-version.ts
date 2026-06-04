/**
 * version/staleness instrument — "which engine is actually live, and
 * is it current?" in a single MCP call.
 *
 * WHY THIS EXISTS: the MCP pins AREST_CLI at startup (server.ts) and
 * re-spawns that exact path on every call. When the startup resolver
 * picks a binary from a different build profile than the one being
 * rebuilt, the server keeps spawning a STALE engine while the operator
 * believes a fresh one is live — undetected, because the only prior
 * check (cli-staleness.ts) compares the resolved exe's mtime to source
 * and is blind to a profile mismatch. The unambiguous fix is the
 * RUNNING binary self-reporting its provenance: `arest-cli version`
 * prints the git SHA + build time it was compiled from, and this
 * module compares that against the repo's current HEAD.
 *
 * `compareEngineVersion` is the pure, unit-testable core (mirrors
 * cli-resolver.ts): given the version-JSON the binary printed and the
 * current HEAD SHA, it returns the comparison shape. The I/O (spawning
 * the CLI, reading git) lives in the verb handler in server.ts.
 */

const UNKNOWN = 'unknown'

export interface EngineVersionComparison {
  /** git SHA the live binary was compiled from (or "unknown"). */
  live_sha: string
  /** build timestamp embedded in the live binary (or "unknown"). */
  live_built: string
  /** cargo package version embedded in the live binary (or "unknown"). */
  pkg: string
  /** repo's current `git rev-parse HEAD` (or "unknown" if git failed). */
  head_sha: string
  /** true ONLY when live_sha and head_sha are both known and equal. */
  up_to_date: boolean
  /** human-readable explanation when not up to date; null when current. */
  behind_message: string | null
}

interface RawVersion {
  sha?: unknown
  built?: unknown
  pkg?: unknown
}

function asString(v: unknown): string {
  return typeof v === 'string' && v.length > 0 ? v : UNKNOWN
}

function shortSha(sha: string): string {
  return sha === UNKNOWN ? UNKNOWN : sha.slice(0, 12)
}

/**
 * Compare the live engine's self-reported version against the repo HEAD.
 *
 * @param versionJson stdout of `arest-cli version` — expected
 *        `{"sha","built","pkg"}`. Malformed input degrades to "unknown"
 *        fields and up_to_date=false (never throws).
 * @param headSha raw `git rev-parse HEAD` output (whitespace tolerated),
 *        or "unknown" when git is unavailable at query time.
 */
export function compareEngineVersion(
  versionJson: string,
  headSha: string,
): EngineVersionComparison {
  let parsed: RawVersion = {}
  try {
    const obj = JSON.parse(versionJson)
    if (obj && typeof obj === 'object') parsed = obj as RawVersion
  } catch {
    // Malformed CLI output (old binary without the `version` verb, an
    // error message, etc.) — fall through to all-unknown.
  }

  const live_sha = asString(parsed.sha)
  const live_built = asString(parsed.built)
  const pkg = asString(parsed.pkg)
  const head_sha = asString(headSha.trim())

  const shaKnown = live_sha !== UNKNOWN && head_sha !== UNKNOWN
  const up_to_date = shaKnown && live_sha === head_sha

  let behind_message: string | null = null
  if (!up_to_date) {
    if (live_sha === UNKNOWN) {
      behind_message =
        `Live engine reports an UNKNOWN git SHA (built ${live_built}). ` +
        `Either the running binary predates the \`version\` subcommand, or git ` +
        `was unavailable when it was built. Rebuild with \`cargo build --bin ` +
        `arest-cli --features local\` and relaunch the MCP so provenance is reportable.`
    } else if (head_sha === UNKNOWN) {
      behind_message =
        `Cannot determine repo HEAD (git unavailable) to compare against the live ` +
        `engine at ${shortSha(live_sha)}. Staleness is indeterminate; verify git ` +
        `is on PATH and the repo is intact.`
    } else {
      behind_message =
        `Live engine is STALE: running ${shortSha(live_sha)} (built ${live_built}) ` +
        `but repo HEAD is ${shortSha(head_sha)}. The MCP pinned this binary at ` +
        `startup, so a rebuild alone will NOT take effect until the server is ` +
        `relaunched. Rebuild with \`cargo build --bin arest-cli --features local\` ` +
        `then restart the MCP server.`
    }
  }

  return { live_sha, live_built, pkg, head_sha, up_to_date, behind_message }
}
