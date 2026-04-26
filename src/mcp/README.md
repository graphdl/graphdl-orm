# AREST MCP Server — Getting Started

A Model Context Protocol server that exposes the AREST engine as tools an
AI agent can call: list and create entities, run state-machine transitions,
verify text against constraints, propose schema changes, and synthesize
prose from facts.

Two modes:

| Mode    | When                                           | Engine               |
|---------|------------------------------------------------|----------------------|
| `local` | personal / private — your readings, your data | bundled WASM, no net |
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
Transition 'place' is from Status 'In Cart' to Status 'Placed'.
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
