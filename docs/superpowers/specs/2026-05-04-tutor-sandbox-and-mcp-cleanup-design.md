# Tutor sandbox + `.mcp.json` cleanup — design

**Date:** 2026-05-04
**Status:** approved (awaiting spec review)
**Owner:** MCP server (`src/mcp/*.ts`)

## Problem

The AREST repo currently exposes its MCP layer through three named entries
in `.mcp.json` (`arest`, `arest-tutor`, `arest-remote`) that all spawn the
same `src/mcp/server.ts`, differing only by `AREST_READINGS_DIR`. The
duplicate `arest-tutor` registration is a leftover from the pre-fold era
when the tutor was its own npm package (folded in commit `238cb89b`,
2026-04-12). It exists today only because the in-tree `tutor` lesson tool
grades its `~~~ expect …` predicates against whatever app is currently
active — so it only works when the active scope is `tutor/domains/`. From
any other scope (e.g. a user's `apps/<name>/` brain) most predicates fail
because the lessons reference nouns like `Order` and `Customer` that do
not exist there.

Two consequences:

1. The "two-server" appearance in `.mcp.json` is cosmetic — one binary,
   different env — but it confuses new users.
2. The tutor is not orthogonal to the active app. You either give up your
   working scope to learn, or you can read lesson narratives but cannot
   make their predicates flip ✓.

## Goal

One MCP server, one user-facing local registration. The tutor runs in an
isolated sandbox engine that is always available, regardless of active
app. A learner can take lessons end-to-end without touching their working
scope, and lesson chains (e.g. E4 depending on `acme-1` from E3) survive
both `tutor.*` calls within a session and MCP server restarts.

## Non-goals

- Refactoring the lesson format itself. Lessons stay as
  `tutor/lessons/<track>/<NN>-*.md` with the existing `~~~ expect …`
  predicate grammar.
- Touching the engine crates or the FORML2 parser. This is purely an MCP
  server change plus a config tidy.
- Migrating the `arest-remote` entry. Worker mode is unrelated.
- Auto-switching the active app on tutor entry/exit. Approach (B) from
  brainstorming was explicitly rejected; the sandbox stays parallel.

## Design

### Two engines, one process

The MCP server keeps two engines side by side inside one process:

| Engine          | Bootstrapped from              | Persisted to                        | Mutated by                |
|-----------------|--------------------------------|-------------------------------------|---------------------------|
| **active app**  | `$AREST_READINGS_DIR` (or app registry) | `$AREST_DB`                  | `apply`, `compile`, `propose` |
| **tutor sandbox** | `tutor/domains/` (in-repo)   | `$AREST_TUTOR_DB` (default `tutor/.sandbox/tutor.db`) | `tutor.apply`, `tutor.compile`, `tutor.propose` |

The two never share state. `tutor.*` writes are not visible to bare `apply`
/ `query` / `list`, and vice versa. The sandbox is lazy-loaded on the first
`tutor.*` call so MCP boot stays cheap for users who never touch it.

### Module layout

New file `src/mcp/tutor-sandbox.ts` exports:

- `getSandbox(): Promise<SandboxHandle>` — returns the lazy-loaded engine
  handle, bootstrapping from `tutor/domains/` on first call.
- `tutorSystemCall(call: string, payload: string): Promise<string>` —
  same shape as the existing `systemCall`, but routed to the sandbox
  engine.
- `resetSandbox(): Promise<void>` — closes the engine, deletes the
  sandbox DB file, re-bootstraps on the next call.

`src/mcp/server.ts` changes:

- Register eight new tools: `tutor.propose`, `tutor.apply`, `tutor.compile`,
  `tutor.query`, `tutor.list`, `tutor.get`, `tutor.actions`, `tutor.reset`.
  Each is a thin wrapper that builds the same payload as its bare
  counterpart and calls `tutorSystemCall`.
- Refactor `evalExpectPredicate` so it accepts a `systemCall` function as
  an argument (defaults to the active-engine call for backward
  compatibility). The existing `tutor` lesson tool passes
  `tutorSystemCall` as the argument, so all predicate grading routes to
  the sandbox.
- The bare `apply`, `query`, `list`, `get`, `compile`, `propose`,
  `actions` tools are unchanged.

### Tool naming

`tutor.*` (with the dot) matches the existing `apps.*` convention
(`apps.current`, `apps.use`, `apps.compile`). The MCP transport renders
dots as part of the tool name; both Claude Code and Codex tolerate the
form.

### Sandbox DB location & lifecycle

Default path: resolved relative to the MCP server source as
`resolve(__dirname, '..', '..', 'tutor', '.sandbox', 'tutor.db')` — i.e.
`<repo-or-package-root>/tutor/.sandbox/tutor.db`. Override via
`$AREST_TUTOR_DB`. Add `tutor/.sandbox/` to `.gitignore` so the file is
never committed. `tutor.reset` deletes the file (closing the engine
first) and the next `tutor.*` call rebootstraps it from
`tutor/domains/`.

This location is chosen over `~/.arest/tutor.db` because it co-locates the
sandbox state with the lessons that produce it — easy to nuke, easy to
inspect during development. Multi-machine sync is out of scope; if a user
wants their tutor progress to follow them, they set `$AREST_TUTOR_DB` to
a synced path.

### `.mcp.json`

Final file:

```json
{
  "$schema": "https://raw.githubusercontent.com/modelcontextprotocol/sdk/main/schemas/mcp.json",
  "mcpServers": {
    "arest": {
      "description": "AREST engine — FORML2 readings → REST API. Tools: apply / compile / propose / query / audit / verify, plus a built-in sandboxed tutor (tutor + tutor.* tools) that runs orthogonal to your active app.",
      "command": "npx",
      "args": ["-y", "tsx", "src/mcp/server.ts"],
      "env": { "AREST_MODE": "local", "AREST_READINGS_DIR": "./readings" }
    },
    "arest-remote": {
      "description": "AREST engine (remote worker mode). Requires AREST_URL and AREST_API_KEY set in your shell.",
      "command": "npx",
      "args": ["-y", "tsx", "src/mcp/server.ts"],
      "env": {
        "AREST_MODE": "remote",
        "AREST_URL": "${AREST_URL}",
        "AREST_API_KEY": "${AREST_API_KEY}"
      }
    }
  }
}
```

The `arest-tutor` entry is dropped. Personal configs that still reference
it continue to spawn a valid server (graceful degradation), but it is no
longer needed.

### Documentation

`tutor/README.md` is updated to:

1. Note that the dedicated `arest-tutor` MCP registration is deprecated —
   the same `arest` entry now exposes the tutor.
2. Document the eight `tutor.*` tools and the sandbox DB location.
3. Describe `tutor.reset` as the way to wipe progress and start fresh.

## Tests (TDD; write first)

New file `src/mcp/tutor-sandbox.test.ts`:

1. **Sandbox boots from `tutor/domains/` on first call.** Calling
   `tutorSystemCall('list:Noun', '')` against a freshly-deleted sandbox
   returns a list containing `Order`, `Customer`, etc.
2. **Sandbox writes invisible to active engine.** `tutor.apply` create
   of a `Customer` named `alice` is visible via `tutor.query`, but the
   bare `query` against an empty active scope sees nothing.
3. **Active writes invisible to sandbox.** With the active engine
   pointed at a minimal test-fixture readings dir (one entity type that
   does not exist in `tutor/domains/`), a bare `apply` create against
   the active engine does not appear in any `tutor.query` /
   `tutor.list` result.
4. **Lesson predicates grade against sandbox.**
   `tutor({ track: 'easy', num: 1 })` returns `check.ok === true` for
   `list Noun contains {"id":"Order"}` even when the active scope is
   empty.
5. **`tutor.reset` wipes sandbox.** After a `tutor.apply` create + a
   `tutor.reset`, the next `tutor.list` shows the noun catalog from
   `tutor/domains/` with no learner-created entities.
6. **Persistence across server restarts.** Two `getSandbox()` instances
   pointing at the same `$AREST_TUTOR_DB` file: a `tutor.apply` from
   instance A is visible from instance B's `tutor.query`. (Critical for
   lesson chains.)

Existing tests for the bare `apply` / `query` / `list` / `tutor` tools
must still pass without modification — the changes are additive.

## Migration & backward compatibility

- `.mcp.json` change: drops a server entry that personal configs may
  pin. The drop is safe — anyone who hard-codes `arest-tutor` keeps a
  working MCP. The README will tell them they don't need it.
- Tool surface: purely additive. Existing tools (`apply`, `tutor`,
  `query`, …) keep their names and schemas.
- Predicate grader signature change is internal to `server.ts`; default
  arg keeps existing call sites compiling.
- Sandbox DB at `tutor/.sandbox/tutor.db` is gitignored and ephemeral
  per developer machine.

## Risks & open questions

- **`tutor/domains/` evolution.** When a domain reading is edited, the
  sandbox DB will diverge from the new schema. The first `tutor.*` call
  after an edit will detect the drift and throw a "schema mismatch — run
  `tutor.reset`" error. Implementation: store a hash of the loaded
  readings in a sandbox metadata table; compare on each open.
- **Concurrent agents.** Two MCP processes pointing at the same
  `$AREST_TUTOR_DB` file is the same SQLite-file-lock problem the active
  engine already has. Out of scope for this design; documented in the
  README.
