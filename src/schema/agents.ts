/**
 * 3NF DDL for AI agent behavioral entities.
 * Derived from readings/agents.md.
 */
export const AGENT_DDL: string[] = [
  `CREATE TABLE IF NOT EXISTS models (
    id TEXT PRIMARY KEY,
    name TEXT,
    code TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE TABLE IF NOT EXISTS agent_definitions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    model_id TEXT REFERENCES models(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_agent_definitions_domain ON agent_definitions(domain_id)`,
  `CREATE INDEX IF NOT EXISTS idx_agent_definitions_model ON agent_definitions(model_id)`,

  `CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    agent_definition_id TEXT NOT NULL REFERENCES agent_definitions(id),
    resource_id TEXT REFERENCES resources(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_agents_definition ON agents(agent_definition_id)`,
  `CREATE INDEX IF NOT EXISTS idx_agents_domain ON agents(domain_id)`,
  `CREATE INDEX IF NOT EXISTS idx_agents_resource ON agents(resource_id)`,

  `CREATE TABLE IF NOT EXISTS completions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id),
    input_text TEXT,
    output_text TEXT,
    occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_completions_agent ON completions(agent_id)`,
  `CREATE INDEX IF NOT EXISTS idx_completions_domain ON completions(domain_id)`,
]
