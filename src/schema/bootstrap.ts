/**
 * Bootstrap DDL for GraphDL metamodel tables.
 *
 * AUTO-GENERATED from readings/*.md — do not edit manually.
 * Regenerate with: npx tsx scripts/generate-bootstrap.ts
 *
 * Source files:
 *   readings/core.md
 *   readings/organizations.md
 *   readings/state.md
 *   readings/instances.md
 *   readings/agents.md
 */

export const BOOTSTRAP_DDL: string[] = [
  `CREATE TABLE IF NOT EXISTS organizations (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  slug TEXT NOT NULL UNIQUE,
  name TEXT
)`,

  `CREATE TABLE IF NOT EXISTS org_memberships (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  user_email TEXT NOT NULL,
  organization_id TEXT NOT NULL REFERENCES organizations(id),
  role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
  UNIQUE(user_email, organization_id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_org_memberships_email ON org_memberships(user_email)`,

  `CREATE INDEX IF NOT EXISTS idx_org_memberships_org ON org_memberships(organization_id)`,

  `CREATE UNIQUE INDEX IF NOT EXISTS idx_org_one_owner ON org_memberships(organization_id) WHERE role = 'owner'`,

  `CREATE TABLE IF NOT EXISTS apps (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  slug TEXT NOT NULL UNIQUE,
  name TEXT,
  app_type TEXT,
  chat_endpoint TEXT,
  organization_id TEXT REFERENCES organizations(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_apps_slug ON apps(slug)`,

  `CREATE INDEX IF NOT EXISTS idx_apps_org ON apps(organization_id)`,

  `CREATE TABLE IF NOT EXISTS domains (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  domain_slug TEXT NOT NULL UNIQUE,
  name TEXT,
  app_id TEXT REFERENCES apps(id),
  organization_id TEXT REFERENCES organizations(id),
  label TEXT,
  visibility TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private', 'public'))
)`,

  `CREATE INDEX IF NOT EXISTS idx_domains_org ON domains(organization_id)`,

  `CREATE INDEX IF NOT EXISTS idx_domains_slug ON domains(domain_slug)`,

  `CREATE TABLE IF NOT EXISTS nouns (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT NOT NULL,
  object_type TEXT NOT NULL DEFAULT 'entity' CHECK (object_type IN ('entity', 'value')),
  super_type_id TEXT REFERENCES nouns(id),
  plural TEXT,
  value_type TEXT,
  format TEXT,
  enum_values TEXT,
  minimum REAL,
  maximum REAL,
  pattern TEXT,
  prompt_text TEXT,
  world_assumption TEXT DEFAULT 'closed' CHECK (world_assumption IN ('closed', 'open')),
  reference_scheme TEXT
)`,

  `CREATE INDEX IF NOT EXISTS idx_nouns_domain ON nouns(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_nouns_name_domain ON nouns(name, domain_id)`,

  `CREATE TABLE IF NOT EXISTS graph_schemas (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT NOT NULL,
  title TEXT
)`,

  `CREATE INDEX IF NOT EXISTS idx_graph_schemas_domain ON graph_schemas(domain_id)`,

  `CREATE TABLE IF NOT EXISTS readings (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  text TEXT NOT NULL,
  graph_schema_id TEXT REFERENCES graph_schemas(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_readings_domain ON readings(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_readings_schema ON readings(graph_schema_id)`,

  `CREATE INDEX IF NOT EXISTS idx_readings_text_domain ON readings(text, domain_id)`,

  `CREATE TABLE IF NOT EXISTS roles (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  reading_id TEXT REFERENCES readings(id),
  noun_id TEXT REFERENCES nouns(id),
  graph_schema_id TEXT REFERENCES graph_schemas(id),
  role_index INTEGER NOT NULL DEFAULT 0,
  name TEXT
)`,

  `CREATE INDEX IF NOT EXISTS idx_roles_reading ON roles(reading_id)`,

  `CREATE INDEX IF NOT EXISTS idx_roles_schema ON roles(graph_schema_id)`,

  `CREATE TABLE IF NOT EXISTS constraints (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  kind TEXT NOT NULL CHECK (kind IN ('UC', 'MC', 'SS', 'XC', 'EQ', 'OR', 'XO', 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', 'AC', 'FC', 'VC')),
  modality TEXT NOT NULL DEFAULT 'Alethic' CHECK (modality IN ('Alethic', 'Deontic')),
  text TEXT,
  set_comparison_argument_length INTEGER
)`,

  `CREATE INDEX IF NOT EXISTS idx_constraints_domain ON constraints(domain_id)`,

  `CREATE TABLE IF NOT EXISTS constraint_spans (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  constraint_id TEXT NOT NULL REFERENCES constraints(id),
  role_id TEXT NOT NULL REFERENCES roles(id),
  subset_autofill INTEGER DEFAULT 0
)`,

  `CREATE INDEX IF NOT EXISTS idx_constraint_spans_constraint ON constraint_spans(constraint_id)`,

  `CREATE INDEX IF NOT EXISTS idx_constraint_spans_role ON constraint_spans(role_id)`,

  `CREATE TABLE IF NOT EXISTS state_machine_definitions (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  title TEXT,
  noun_id TEXT REFERENCES nouns(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_smd_domain ON state_machine_definitions(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_smd_noun ON state_machine_definitions(noun_id)`,

  `CREATE TABLE IF NOT EXISTS statuses (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT NOT NULL,
  state_machine_definition_id TEXT NOT NULL REFERENCES state_machine_definitions(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_statuses_smd ON statuses(state_machine_definition_id)`,

  `CREATE TABLE IF NOT EXISTS event_types (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT NOT NULL
)`,

  `CREATE INDEX IF NOT EXISTS idx_event_types_domain ON event_types(domain_id)`,

  `CREATE TABLE IF NOT EXISTS transitions (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  from_status_id TEXT NOT NULL REFERENCES statuses(id),
  to_status_id TEXT NOT NULL REFERENCES statuses(id),
  event_type_id TEXT REFERENCES event_types(id),
  verb_id TEXT REFERENCES verbs(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_transitions_from ON transitions(from_status_id)`,

  `CREATE INDEX IF NOT EXISTS idx_transitions_to ON transitions(to_status_id)`,

  `CREATE TABLE IF NOT EXISTS guards (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT,
  transition_id TEXT REFERENCES transitions(id),
  graph_schema_id TEXT REFERENCES graph_schemas(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_guards_transition ON guards(transition_id)`,

  `CREATE TABLE IF NOT EXISTS verbs (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT NOT NULL,
  status_id TEXT REFERENCES statuses(id),
  transition_id TEXT REFERENCES transitions(id),
  graph_id TEXT REFERENCES graphs(id),
  agent_definition_id TEXT REFERENCES agent_definitions(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_verbs_domain ON verbs(domain_id)`,

  `CREATE TABLE IF NOT EXISTS functions (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT,
  callback_url TEXT,
  http_method TEXT DEFAULT 'POST',
  headers TEXT,
  verb_id TEXT REFERENCES verbs(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_functions_verb ON functions(verb_id)`,

  `CREATE TABLE IF NOT EXISTS streams (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT NOT NULL
)`,

  `CREATE TABLE IF NOT EXISTS models (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT,
  code TEXT
)`,

  `CREATE TABLE IF NOT EXISTS agent_definitions (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT NOT NULL,
  model_id TEXT REFERENCES models(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_agent_definitions_domain ON agent_definitions(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_agent_definitions_model ON agent_definitions(model_id)`,

  `CREATE TABLE IF NOT EXISTS agents (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  agent_definition_id TEXT NOT NULL REFERENCES agent_definitions(id),
  resource_id TEXT REFERENCES resources(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_agents_definition ON agents(agent_definition_id)`,

  `CREATE INDEX IF NOT EXISTS idx_agents_domain ON agents(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_agents_resource ON agents(resource_id)`,

  `CREATE TABLE IF NOT EXISTS completions (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  agent_id TEXT NOT NULL REFERENCES agents(id),
  input_text TEXT,
  output_text TEXT,
  occurred_at TEXT NOT NULL DEFAULT (datetime('now'))
)`,

  `CREATE INDEX IF NOT EXISTS idx_completions_agent ON completions(agent_id)`,

  `CREATE INDEX IF NOT EXISTS idx_completions_domain ON completions(domain_id)`,

  `CREATE TABLE IF NOT EXISTS citations (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  text TEXT NOT NULL,
  uri TEXT,
  retrieval_date TEXT
)`,

  `CREATE INDEX IF NOT EXISTS idx_citations_domain ON citations(domain_id)`,

  `CREATE TABLE IF NOT EXISTS graphs (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  graph_schema_id TEXT REFERENCES graph_schemas(id),
  is_done INTEGER NOT NULL DEFAULT 0
)`,

  `CREATE INDEX IF NOT EXISTS idx_graphs_domain ON graphs(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_graphs_schema ON graphs(graph_schema_id)`,

  `CREATE TABLE IF NOT EXISTS graph_citations (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  graph_id TEXT NOT NULL REFERENCES graphs(id),
  citation_id TEXT NOT NULL REFERENCES citations(id),
  UNIQUE(graph_id, citation_id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_graph_citations_graph ON graph_citations(graph_id)`,

  `CREATE INDEX IF NOT EXISTS idx_graph_citations_citation ON graph_citations(citation_id)`,

  `CREATE TABLE IF NOT EXISTS resources (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  noun_id TEXT REFERENCES nouns(id),
  reference TEXT,
  value TEXT,
  created_by TEXT
)`,

  `CREATE INDEX IF NOT EXISTS idx_resources_domain ON resources(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_resources_noun ON resources(noun_id)`,

  `CREATE TABLE IF NOT EXISTS resource_roles (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  graph_id TEXT NOT NULL REFERENCES graphs(id),
  resource_id TEXT NOT NULL REFERENCES resources(id),
  role_id TEXT NOT NULL REFERENCES roles(id),
  UNIQUE(graph_id, role_id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_resource_roles_graph ON resource_roles(graph_id)`,

  `CREATE TABLE IF NOT EXISTS state_machines (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT,
  state_machine_definition_id TEXT REFERENCES state_machine_definitions(id),
  current_status_id TEXT REFERENCES statuses(id),
  resource_id TEXT REFERENCES resources(id)
)`,

  `CREATE INDEX IF NOT EXISTS idx_state_machines_domain ON state_machines(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_state_machines_definition ON state_machines(state_machine_definition_id)`,

  `CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  event_type_id TEXT REFERENCES event_types(id),
  state_machine_id TEXT REFERENCES state_machines(id),
  graph_id TEXT REFERENCES graphs(id),
  data TEXT,
  occurred_at TEXT NOT NULL DEFAULT (datetime('now'))
)`,

  `CREATE INDEX IF NOT EXISTS idx_events_state_machine ON events(state_machine_id)`,

  `CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type_id)`,

  `CREATE TABLE IF NOT EXISTS guard_runs (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version INTEGER NOT NULL DEFAULT 1,
  name TEXT,
  guard_id TEXT REFERENCES guards(id),
  graph_id TEXT REFERENCES graphs(id),
  result INTEGER
)`,

  `CREATE INDEX IF NOT EXISTS idx_guard_runs_guard ON guard_runs(guard_id)`,

  `CREATE TABLE IF NOT EXISTS generators (
  id TEXT PRIMARY KEY,
  domain_id TEXT REFERENCES domains(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  version_num INTEGER NOT NULL DEFAULT 1,
  output_format TEXT NOT NULL DEFAULT 'openapi',
  title TEXT,
  version TEXT DEFAULT '1.0',
  output TEXT
)`,

  `CREATE INDEX IF NOT EXISTS idx_generators_domain ON generators(domain_id)`,

  `CREATE INDEX IF NOT EXISTS idx_generators_format ON generators(output_format)`,
]
