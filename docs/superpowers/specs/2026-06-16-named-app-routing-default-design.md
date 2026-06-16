# Named app routing as the default (stateless multi-app invocation)

**Date:** 2026-06-16
**Status:** Design — verbally approved; pending written-spec review
**Task:** `mcp-multi-app-scoping` (p1, in_progress)
**Scope:** `arest-987/src/mcp/server.ts` (the TypeScript MCP server). No engine/Rust changes.

## Problem

Sub-agents share ONE stdio connection to the MCP server with no per-session id. The
server keys app selection off a single global `activeApp` plus an on-disk
`.arest-active-app` marker. So one agent's `apps_use` silently re-scopes every other
agent's reads and writes — "multi-app clobbering."

It also caused a concrete operator bug. `orient` reads whatever app is globally active
and ignores any intent to look at a different app, so an `orient` issued while believing
the active app was X actually reported Y's state (the wrong-app 0-counts incident that
cost several wasted ticks and a reverted "fix").

Per-connection isolation is not viable (shared stdio, no session id). The fix is to make
each CALL carry its own app scope.

## Goal / Non-goals

**Goal.** Make named per-call `app` routing the default, primary way to invoke every
verb, so multiple agents share the substrate without clobbering — "no state with the
interface, as Codd intended."

**Non-goals.**
- Removing the global active-app. It is *kept* as a mutable default fallback for
  omitted-`app` calls (a deliberate user decision; see Trade-off).
- Per-connection / per-session server state (not viable on shared stdio).
- Unrelated bugs: orient's "ready" bin not matching the "pending" status; stage-2
  `Task_has_Task_Status` staleness; delta occ-3. Tracked separately (Out of scope).

## Key insight: the routing infrastructure already exists

Most of this fix is already built — `resolveCallScope` was added for exactly this
purpose. What remains is small and mechanical.

- **`resolveCallScope(app)`** (server.ts:271) — PURE: resolves an app name to
  `{name, dbPath, readingsDir, exists}` via the pure `resolveArestApp` registry.
  Documented invariants (244–254): never assigns `activeApp`, never writes the marker;
  per-call engine handles live in a separate `_perCallHandles` cache so two apps coexist
  without invalidating each other. **`callScope(app?)`** (348) wraps it, returning
  `undefined` when `app` is omitted (⇒ global fallback).
- **`systemCall(key, input, scope?)`** (543), **`dispatchCommand(cmd, scope?)`** (516),
  **`dispatchRead(path, scope?)`** (533) already thread an optional scope: scoped → CLI
  runs against `currentDbPath(scope)` / engine uses `currentReadingsDir(scope)` + the
  per-call handle; omitted → global fallback, byte-for-byte unchanged.
- **Every data verb already routes this way:** query (1391), sql (1451), cells (1523),
  get/apply (1193), retract (2011) each do `const scope = callScope(app); … systemCall(…, scope)`.
- **The context-receipt is already app-agnostic:** minted against the global (897) and
  explicitly NOT re-scoped by a per-call `app` (877–882) — "routing convenience, not a
  scope escalation."

What is missing: (1) the state-touching verbs that drop the scope — chiefly `orient` —
and (2) the framing: making per-call `app` the documented primary and demoting the
global to a named default.

## Design

### 1. Thread the scope into every state-touching verb (the real code gap)

**orient** — the known gap and the bug. Today (1897–1910) it takes `active_app` as a
label only and calls `systemCall('orient', envelope)` with no scope, so it always reads
the global active DB. Change:
- Add `app: z.string().optional()` to orient's inputSchema. `app` is the do-everything
  primary: it both **routes** the read (`const scope = callScope(app)`) and **labels**
  the active entry + suggested_next (`envelope.active_app = scope ? scope.name : activeApp.name`).
- Thread the scope: `systemCall('orient', JSON.stringify(envelope), scope)`.
- `active_app` is kept but demoted to a **deprecated, label-only fallback**: it is honored
  for the label iff `app` is omitted, and it never routes (preserving old callers
  byte-for-byte). Its description marks it deprecated in favor of `app`.
- Result: `orient app="tasks"` reads the tasks snapshot regardless of the global —
  multi-agent-safe and the direct fix for the wrong-app incident. `orient` with neither
  param falls back to the global for both routing and label, exactly as today.

**Audit the remaining verbs** and patch any that issue a `systemCall` /
`dispatchRead` / `dispatchCommand` for app state *without* threading a scope. Candidates
to check: `apps_compile`, `apps_status`, `apps_check`, `actions`, `explain`, `schema`,
`validate`, `verify`, `induce`, `propose`, `synthesize`, `ask`, `select_component`. Each
that reads/mutates app state gains `app` + threads `scope`, mirroring the data verbs.
Inherently-global verbs (`apps_list`, `engine_version`) are exempt and labelled as such.

### 2. Per-call `app` is the documented primary

Every routed verb's description states: pass `app=<name>` to route this call to that app
statelessly — the multi-agent-safe default. Rewrite orient's description to drop the
"counts come from the ACTIVE app's loaded snapshot only" gotcha and replace it with
"pass `app` to route counts to that app."

### 3. Omitted `app` → the mutable global default (kept)

When `app` is omitted, every helper falls back to `activeApp` exactly as today.
Single-app sessions need not pass `app`. This is byte-for-byte the current behavior.

### 4. Repurpose `apps_use` to set the process default

`apps_use` keeps its mechanism (set `activeApp` + persist the `.arest-active-app`
marker) but is *reframed*: it sets the process's DEFAULT app — the fallback used only
when a call omits `app` — not "the one active app." Concretely:
- Description: from "make this the active UoD for subsequent operations" to "set the
  default app for calls that omit `app`; prefer passing `app` per call for multi-agent
  safety." The marker is the persisted default (seeded at startup from `AREST_APP`).
- `apps_current` reports "the default app" rather than "the active app."
- `appSummary`'s next-action nudge (385–391) is repointed from `apps.use` to a per-call
  `app` hint ("pass `app=<name>` to route a call to this app").

### 5. context-receipt stays app-agnostic

No code change. Confirm the existing note (877–882) holds and that no verb re-scopes or
invalidates a receipt on a per-call `app`. Document that one receipt is valid across any
`app` routing.

## Behavior summary

- **Multi-agent:** each agent passes `app` on every call → stateless routing through
  `resolveCallScope`, separate per-call handles, no shared mutation → no clobbering.
- **Single-app:** omit `app`; calls use the global default (set once via
  `apps_use` / `AREST_APP`).

## Trade-off (explicit, accepted)

Keeping a *mutable* global default means a multi-agent caller that OMITS `app` and leans
on the default can still collide with another agent's `apps_use`. Making per-call `app`
the documented primary *mitigates* but does not *eliminate* this. Full elimination would
require removing the global default and requiring `app` on every call — deliberately not
done, to preserve single-app ergonomics and backward compatibility. The residual risk is
bounded to omitted-`app` calls and is the user's accepted position.

## Testing

- **Per-call routing reads the named app regardless of the global** — unit test with a
  fixture `appsDir`: set global to A, call with `app=B`, assert B's data is returned and
  `activeApp` / marker are unchanged (purity).
- **orient regression** — with global active = claude, `orient app="tasks"` returns
  tasks counts (the exact incident); `orient` with no `app` returns the global's counts.
- **Omitted `app` → default** — calls with no `app` hit the global, unchanged.
- **apps_use sets the default without disturbing a concurrent per-call `app`** —
  `apps_use X`; a concurrent `query app=Y` still reads Y.
- **resolveCallScope purity preserved** — assert no `activeApp` assignment / marker write
  on any per-call path (guards the core invariant).

## Component / file changes

- `arest-987/src/mcp/server.ts`:
  - **orient**: inputSchema `+app`; thread `scope` into `systemCall('orient', …)`;
    description rewrite.
  - **audited aux verbs**: `+app` and thread `scope` wherever they read/mutate app state
    without it today.
  - **apps_use / apps_current / appSummary**: reframe descriptions + next-action nudges
    (default vs primary).
  - **verb-description sweep**: per-call `app` documented as the primary, multi-agent-safe
    path across routed verbs.
- Tests alongside the existing server tests.
- No Rust/engine changes. No schema/migration changes.

## Out of scope (tracked separately)

- orient "ready" bin vs "pending" status mismatch (separate minor orient bug).
- stage-2 `Task_has_Task_Status` staleness (recommendation-derivation-stale-on-mutation).
- delta occ-3 (the ns-domain dynamic-read rules; the 82.5→8.0 bulk win).
- Removing the global default entirely (a possible follow-up if omitted-`app` multi-agent
  collisions prove real in practice).
