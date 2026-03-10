/**
 * 3NF DDL for state machine behavioral entities.
 * Derived from readings/state.md + readings/core.md.
 */
export const STATE_DDL: string[] = [
  `CREATE TABLE IF NOT EXISTS state_machine_definitions (
    id TEXT PRIMARY KEY,
    title TEXT,
    noun_id TEXT REFERENCES nouns(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_smd_domain ON state_machine_definitions(domain_id)`,
  `CREATE INDEX IF NOT EXISTS idx_smd_noun ON state_machine_definitions(noun_id)`,

  `CREATE TABLE IF NOT EXISTS statuses (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    state_machine_definition_id TEXT NOT NULL REFERENCES state_machine_definitions(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_statuses_smd ON statuses(state_machine_definition_id)`,

  `CREATE TABLE IF NOT EXISTS event_types (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_event_types_domain ON event_types(domain_id)`,

  `CREATE TABLE IF NOT EXISTS transitions (
    id TEXT PRIMARY KEY,
    from_status_id TEXT NOT NULL REFERENCES statuses(id),
    to_status_id TEXT NOT NULL REFERENCES statuses(id),
    event_type_id TEXT REFERENCES event_types(id),
    verb_id TEXT REFERENCES verbs(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_transitions_from ON transitions(from_status_id)`,
  `CREATE INDEX IF NOT EXISTS idx_transitions_to ON transitions(to_status_id)`,

  `CREATE TABLE IF NOT EXISTS guards (
    id TEXT PRIMARY KEY,
    name TEXT,
    transition_id TEXT REFERENCES transitions(id),
    graph_schema_id TEXT REFERENCES graph_schemas(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_guards_transition ON guards(transition_id)`,

  `CREATE TABLE IF NOT EXISTS verbs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status_id TEXT REFERENCES statuses(id),
    transition_id TEXT REFERENCES transitions(id),
    graph_id TEXT REFERENCES graphs(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_verbs_domain ON verbs(domain_id)`,

  `CREATE TABLE IF NOT EXISTS functions (
    id TEXT PRIMARY KEY,
    name TEXT,
    callback_url TEXT,
    http_method TEXT DEFAULT 'POST',
    verb_id TEXT REFERENCES verbs(id),
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,

  `CREATE INDEX IF NOT EXISTS idx_functions_verb ON functions(verb_id)`,

  `CREATE TABLE IF NOT EXISTS streams (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain_id TEXT REFERENCES domains(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    version INTEGER NOT NULL DEFAULT 1
  )`,
]
