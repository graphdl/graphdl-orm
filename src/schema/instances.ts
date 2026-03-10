/**
 * 3NF DDL for runtime instance entities.
 * Derived from readings/instances.md.
 */
export const INSTANCE_DDL: string[] = [
  `CREATE TABLE IF NOT EXISTS graphs (
    id TEXT PRIMARY KEY,
    graph_schema_id TEXT REFERENCES graph_schemas(id),
    domain_id TEXT REFERENCES domains(id),
    is_done INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_graphs_domain ON graphs(domain_id)`,
  `CREATE INDEX IF NOT EXISTS idx_graphs_schema ON graphs(graph_schema_id)`,

  `CREATE TABLE IF NOT EXISTS resources (
    id TEXT PRIMARY KEY,
    noun_id TEXT REFERENCES nouns(id),
    reference TEXT,
    value TEXT,
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_resources_domain ON resources(domain_id)`,
  `CREATE INDEX IF NOT EXISTS idx_resources_noun ON resources(noun_id)`,

  `CREATE TABLE IF NOT EXISTS resource_roles (
    id TEXT PRIMARY KEY,
    graph_id TEXT NOT NULL REFERENCES graphs(id),
    resource_id TEXT NOT NULL REFERENCES resources(id),
    role_id TEXT NOT NULL REFERENCES roles(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1,
    UNIQUE(graph_id, role_id)
  )`,

  `CREATE INDEX IF NOT EXISTS idx_resource_roles_graph ON resource_roles(graph_id)`,

  `CREATE TABLE IF NOT EXISTS state_machines (
    id TEXT PRIMARY KEY,
    name TEXT,
    state_machine_definition_id TEXT REFERENCES state_machine_definitions(id),
    current_status_id TEXT REFERENCES statuses(id),
    resource_id TEXT REFERENCES resources(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_state_machines_domain ON state_machines(domain_id)`,
  `CREATE INDEX IF NOT EXISTS idx_state_machines_definition ON state_machines(state_machine_definition_id)`,

  `CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    event_type_id TEXT REFERENCES event_types(id),
    state_machine_id TEXT REFERENCES state_machines(id),
    graph_id TEXT REFERENCES graphs(id),
    data TEXT,
    occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_events_state_machine ON events(state_machine_id)`,
  `CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type_id)`,

  `CREATE TABLE IF NOT EXISTS guard_runs (
    id TEXT PRIMARY KEY,
    name TEXT,
    guard_id TEXT REFERENCES guards(id),
    graph_id TEXT REFERENCES graphs(id),
    result INTEGER,
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_guard_runs_guard ON guard_runs(guard_id)`,
]
