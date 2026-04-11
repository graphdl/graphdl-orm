# AREST API

## CLI (local mode)

The `arest` binary is the SYSTEM function. Every call is `system(key, input)`.

```bash
# Compile readings into database
arest <readings_dir> --db app.db

# Single SYSTEM call
arest <key> <input> --db app.db

# REPL mode
arest --db app.db
```

### Keys

| Key | Input | Description |
|-----|-------|-------------|
| `compile` | FORML2 markdown text | Self-modification: parse, merge, recompile DEFS |
| `create:<Noun>` | `<<field, value>, ...>` | Create entity with fact pairs |
| `update:<Noun>` | `<<id, val>, <field, val>, ...>` | Update entity fields |
| `transition:<Noun>` | `<entity_id, event>` | Fire SM transition |
| `transitions:<Noun>` | status atom | Query available transitions from a status |
| `debug` | (empty) | Dump compiled state (requires debug-def feature) |

### Examples

```bash
# Create an Order
arest "create:Order" "<<id, ord-1>, <customer, acme>>" --db orders.db

# Check transitions from Draft
arest "transitions:Order" "Draft" --db orders.db

# Fire a transition
arest "transition:Order" "<ord-1, place>" --db orders.db

# Self-modify: load new readings
arest compile "Person(.Name) is an entity type. Person has Email." --db app.db
```

## MCP Server

The MCP server exposes the engine as tools for AI agents.

### Tools

| Tool | Description |
|------|-------------|
| `graphdl_create` | Create entity (resolve -> derive -> validate -> emit) |
| `graphdl_list` | List entities of a noun type |
| `graphdl_get` | Get entity by ID |
| `graphdl_transition` | Fire SM transition |
| `graphdl_compile` | Self-modification: ingest FORML2 readings |
| `graphdl_evaluate` | Run constraint evaluation |
| `graphdl_schema` | Get domain schema |
| `graphdl_apply` | Generic command dispatch |
| `graphdl_audit_log` | Read audit trail |
| `graphdl_verify_signature` | Verify HMAC-SHA256 signature |

### Configuration

```json
{
  "mcpServers": {
    "graphdl": {
      "command": "npx",
      "args": ["-y", "graphdl-orm", "mcp"],
      "env": {
        "GRAPHDL_MODE": "local",
        "GRAPHDL_READINGS_DIR": "/path/to/readings"
      }
    }
  }
}
```

## HTTP API (remote mode)

Base URL: set via `GRAPHDL_URL` env var.

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/arest/chat` | POST | Conversational agent with constraint-checked draft loop |
| `/arest/extract` | POST | LLM fact population (OWA populating function) |
| `/arest/domains` | GET/POST | List or create domains |
| `/arest/domains/:slug` | GET | Get domain |
| `/arest/domains/:slug/readings` | GET/POST | List or create readings |
| `/arest/domains/:slug/generate` | POST | Generate output |
| `/arest/parse` | POST | FORML2 parser |
| `/arest/verify` | POST | CSDP validation |

### Identity & Signing

Commands accept optional `sender` and `signature` fields. The sender is pushed as a User fact during resolve. The signature is HMAC-SHA256 over `sender::payload`, verified via `AREST_HMAC_KEY` env var. Omit both to skip identity enforcement.
