# AREST MCP Server — Getting Started

A Model Context Protocol server that exposes the AREST engine as tools an
AI agent can call: list and create entities, run state-machine transitions,
verify text against constraints, propose schema changes, and synthesize
prose from facts.

Two modes:

| Mode    | When                                           | Engine               |
|---------|------------------------------------------------|----------------------|
| `local` | personal / private — your readings, your data | bundled WASM or local SQLite CLI, no net |
| `remote`| against a deployed worker                      | HTTP to `AREST_URL`  |

## Prerequisites

- Node 18+ (npm, yarn, or `npx -y` works)
- For local mode: nothing else — the WASM engine ships in the npm package
- For remote mode: a deployed AREST Worker (see [`docs/cloud.md`](../../docs/cloud.md))

## Quick start (local mode, no install)

```bash
mkdir my-app && cd my-app
mkdir readings
cat > readings/orders.md <<'EOF'
## Entity Types
Order(.Order Id) is an entity type.
Customer(.Name) is an entity type.

## Fact Types
Order was placed by Customer.
  Each Order was placed by exactly one Customer.

## State Machines
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
EOF

AREST_MODE=local AREST_READINGS_DIR=$PWD/readings npx -y arest mcp
```

The server speaks MCP over stdio. To talk to it, plug it into any MCP-aware
client.

## Plug into Claude Desktop / Claude Code

Edit `~/.config/Claude/claude_desktop_config.json` (or the equivalent on
your platform):

```json
{
  "mcpServers": {
    "arest": {
      "command": "npx",
      "args": ["-y", "arest", "mcp"],
      "env": {
        "AREST_MODE": "local",
        "AREST_READINGS_DIR": "/absolute/path/to/your/readings"
      }
    }
  }
}
```

Restart the client. The agent now has access to the AREST verb set.

## Local app scope

An AREST app is a local scope: one app root, one readings directory, and one
SQLite DB. The selected app is the active UoD for local fact operations.
The registry also discovers sibling AREST packages whose `package.json`
declares `kind: "app"` or `kind: "library"`; this is how reusable packages
such as law libraries appear without needing to live under `AREST_APPS_DIR`.
App and library discovery scans the filesystem on each tool call, so new or
changed readings do not require an MCP restart. Restart is only needed after
changing the MCP server code or tool definitions.

Use app mode by setting `AREST_APPS_DIR` and `AREST_APP`, or by setting
explicit `AREST_READINGS_DIR` / `AREST_DB` paths. Existing DB names are
preserved: if an app directory contains exactly one `.db` file, the MCP
server uses it instead of inventing a new name.

```json
{
  "mcpServers": {
    "arest": {
      "command": "npx",
      "args": ["-y", "arest", "mcp"],
      "env": {
        "AREST_MODE": "local",
        "AREST_APPS_DIR": "/absolute/path/to/apps",
        "AREST_APP": "support"
      }
    }
  }
}
```

The app tools are:

- `apps.current` — show the active app, readings directory, DB path, health, and next actions.
- `apps.list` — list apps under `AREST_APPS_DIR`.
- `apps.status` — inspect one app, defaulting to the active app, with reading/DB freshness.
- `apps.check` — summarize health across every discovered app.
- `apps.register` — scan the apps directory and return the directory-derived catalog; it writes no catalog facts.
- `apps.use` — switch the active app; subsequent local operations use that DB.
- `apps.create` — create an app readings directory and optionally compile it.
- `apps.compile` — compile an app's readings into its SQLite DB.

Most MCP clients display dotted tool names with underscores. For example,
`apps.current` may appear as `apps_current`, and `apps.check` may appear as
`apps_check`.

App health statuses are:

- `ready` — readings and DB are present, and the DB is at least as new as the app readings and dependency readings.
- `library` — the directory contains reusable readings or source material but no app marker or DB; this is not an issue and is not an active UoD target.
- `needs_compile` — readings exist but the DB file is missing.
- `stale_db` — one or more app readings or package dependency readings are newer than the DB.
- `missing_readings` — the readings directory is missing or has no `.md` files.
- `not_found` — an explicitly requested app/library name has no root directory.

When an app package declares AREST package dependencies with `file:`
specifiers, app health also includes the direct dependency list, transitive
dependency closure, newest dependency reading timestamp, and whether that
dependency set makes the app DB stale. This keeps compile impact derived from
the filesystem and package graph instead of storing duplicate app-health facts.

## Mutation context gate

Fact-mutating tools are gated by the AREST modeling context. Before calling
`apply`, `compile`, or `propose`, call `context` and pass the returned
`receipt` as `context_receipt` on the mutating call. This forces agents to
load the rules, prompt manifest, app scope, and anti-patterns for typed
FORML facts instead of writing ad hoc prose memory.

The selected app and active fact storage medium are the implicit Universe of
Discourse; the gate does not add a `universe_of_discourse` field or require
UoD meta-facts.

Read-only tools such as `schema`, `get`, `query`, `actions`, `tutor`, `ask`,
`synthesize`, and `validate` remain open so an agent can inspect the model
before taking a write.

## Plug into a remote worker

```json
{
  "mcpServers": {
    "arest": {
      "command": "npx",
      "args": ["-y", "arest", "mcp"],
      "env": {
        "AREST_MODE": "remote",
        "AREST_URL": "https://arest.example.workers.dev",
        "AREST_API_KEY": "your-api-key"
      }
    }
  }
}
```

## Build from source (if hacking on the server)

```bash
git clone https://github.com/graphdl/arest
cd arest
yarn install
yarn build:wasm                     # only needed for local mode
AREST_MODE=local AREST_READINGS_DIR=$PWD/readings yarn mcp
```

## The verb set

| Group          | Verbs                                                      |
|----------------|------------------------------------------------------------|
| Context        | `context`                                                  |
| Apps           | `apps.current`, `apps.list`, `apps.status`, `apps.check`, `apps.register`, `apps.use`, `apps.create`, `apps.compile` |
| Algebra        | `assert`, `retract`, `project`, `compile`                  |
| Entity sugar   | `get`, `query`, `apply`, `create`, `read`, `update`, `transition`, `delete` |
| Introspection  | `explain`, `actions`, `schema`, `verify`                   |
| Evolution      | `propose`, `compile`                                        |
| LLM bridge     | `ask`, `synthesize`, `validate`                             |
| ChatGPT compat | `search`, `fetch`                                           |

Full contract: [`docs/09-mcp-verbs.md`](../../docs/09-mcp-verbs.md).

## Use the checker from an agent

The agent can call:

- `validate` — pass raw text; the LLM extracts candidate facts and the
  engine runs the full constraint check, returning any violations.
- `verify` — pass an entity id or fact-set; the engine runs the deontic +
  alethic gate against the current population.
- `explain` — pass a noun + id; returns the constraints that apply, which
  fired, and which would fire on a hypothetical mutation.

Each violation comes back as a structured `Violation` cell with a pointer
to the offending fact and the rule that fired (Theorem 4) — so the agent
can follow the link, read the rule, and propose a fix without human help.

## Debugging

```bash
AREST_DEBUG=1 npx -y arest mcp     # logs verb dispatch + raw engine responses
```

## Where next

- [`docs/09-mcp-verbs.md`](../../docs/09-mcp-verbs.md) — verb-by-verb reference
- [`docs/cli.md`](../../docs/cli.md) — same engine, terminal-only
- [`docs/cloud.md`](../../docs/cloud.md) — back the MCP server with a deployed worker
- `src/mcp/server.ts` — server entry; tools registered with `server.registerTool(...)`
