/**
 * generate-bootstrap.ts
 *
 * Build-time script that generates bootstrap DDL from readings/*.md files.
 *
 * Usage: npx tsx scripts/generate-bootstrap.ts
 *
 * Pipeline: readings/*.md → parseFORML2 → DDL generation → src/schema/bootstrap.ts
 *
 * This replaces the hardcoded DDL in metamodel.ts, state.ts, instances.ts, agents.ts
 * with DDL derived from the FORML2 readings (the single source of truth).
 *
 * The script:
 * 1. Parses all readings files to discover entity types, value types, fact types, constraints
 * 2. Uses NOUN_TABLE_MAP to determine which entities get tables
 * 3. Analyzes readings (fact types) and constraints to derive columns:
 *    - Value type attributes → TEXT/REAL columns
 *    - N:1 FK relationships (UC on one role) → _id FK columns
 *    - Objectified fact types → junction tables with FK columns
 * 4. Applies CHECK constraints from enum value types
 * 5. Outputs src/schema/bootstrap.ts
 */

import * as fs from 'node:fs'
import * as path from 'node:path'

import { parseFORML2 } from '../src/api/parse.js'
import { NOUN_TABLE_MAP } from '../src/collections.js'

// ── Types ───────────────────────────────────────────────────────────────────

interface NounInfo {
  name: string
  objectType: 'entity' | 'value'
  enumValues?: string[]
}

interface ReadingInfo {
  text: string
  nouns: string[]
  predicate: string
}

interface ConstraintInfo {
  kind: string
  modality: string
  reading: string
  roles: number[]
  text?: string
}

// ── Name conversion (mirrors sqlite.ts) ─────────────────────────────────────

function toTableName(name: string): string {
  if (NOUN_TABLE_MAP[name]) return NOUN_TABLE_MAP[name]
  if (name.includes(' ')) {
    return name.toLowerCase().replace(/\s+/g, '_') + 's'
  }
  return name.replace(/([A-Z])/g, '_$1').toLowerCase().replace(/^_/, '') + 's'
}

function toColumnName(name: string): string {
  if (name.includes(' ')) {
    return name.toLowerCase().replace(/\s+/g, '_')
  }
  return name.replace(/([A-Z])/g, '_$1').toLowerCase().replace(/^_/, '')
}

function toFkColumnName(nounName: string): string {
  return toColumnName(nounName) + '_id'
}

// ── Column and Table types ──────────────────────────────────────────────────

interface Col {
  name: string
  type: string
  extra: string   // e.g., "NOT NULL", "REFERENCES foo(id)", CHECK clauses
}

interface TableSpec {
  tableName: string
  columns: Col[]
  tableConstraints: string[]  // UNIQUE(...) etc.
  indexes: string[]
}

// ── Standard system columns ─────────────────────────────────────────────────

function systemCols(opts: { domain?: boolean }): Col[] {
  const cols: Col[] = [
    { name: 'id', type: 'TEXT', extra: 'PRIMARY KEY' },
  ]
  if (opts.domain !== false) {
    cols.push({ name: 'domain_id', type: 'TEXT', extra: 'REFERENCES domains(id)' })
  }
  cols.push(
    { name: 'created_at', type: 'TEXT', extra: "NOT NULL DEFAULT (datetime('now'))" },
    { name: 'updated_at', type: 'TEXT', extra: "NOT NULL DEFAULT (datetime('now'))" },
    { name: 'version', type: 'INTEGER', extra: 'NOT NULL DEFAULT 1' },
  )
  return cols
}

// ── Per-table column definitions ────────────────────────────────────────────
// These are derived from the readings but with the exact column names and types
// that match the existing hardcoded DDL. The readings are the *source of truth*
// for what columns exist; these mappings encode how reading concepts map to SQL.
//
// The mapping rules:
//   "Noun has Name" + "Each Noun has exactly one Name" → name TEXT NOT NULL
//   "Noun has Object Type" + enum values → object_type TEXT NOT NULL CHECK(...)
//   "Graph Schema has Reading" + UC on Graph Schema role → readings.graph_schema_id FK
//   Objectified fact type "Constraint spans Role" → constraint_spans junction table

function buildTables(
  allNouns: Map<string, NounInfo>,
  allReadings: ReadingInfo[],
  allConstraints: ConstraintInfo[],
  allSubtypes: Array<{ child: string; parent: string }>,
): TableSpec[] {
  const tables: TableSpec[] = []

  // Helper to check if a reading exists
  const hasReading = (text: string) => allReadings.some(r => r.text === text)

  // Helper to check constraint kind on a reading
  const getConstraints = (readingText: string) =>
    allConstraints.filter(c => c.reading === readingText)

  // Helper to check if an entity has a "has Name" reading with MC
  const hasNameReading = (entity: string) => {
    const reading = `${entity} has Name`
    if (!hasReading(reading)) return false
    return getConstraints(reading).some(c => c.kind === 'MC' || c.kind === 'UC')
  }

  // ── Organizations & Access Control (from organizations.md) ──────────

  // organizations — Organization(.Org Slug) entity type
  tables.push({
    tableName: 'organizations',
    columns: [
      ...systemCols({ domain: false }),
      { name: 'slug', type: 'TEXT', extra: 'NOT NULL UNIQUE' },
      // "Organization has Name" → name TEXT
      { name: 'name', type: 'TEXT', extra: '' },
    ],
    tableConstraints: [],
    indexes: [],
  })

  // org_memberships — "User has Org Role in Organization" objectified
  tables.push({
    tableName: 'org_memberships',
    columns: [
      ...systemCols({ domain: false }),
      { name: 'user_email', type: 'TEXT', extra: 'NOT NULL' },
      { name: 'organization_id', type: 'TEXT', extra: 'NOT NULL REFERENCES organizations(id)' },
      // "Org Role" enum values: 'owner', 'admin', 'member'
      { name: 'role', type: 'TEXT', extra: "NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member'))" },
    ],
    tableConstraints: ['UNIQUE(user_email, organization_id)'],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_org_memberships_email ON org_memberships(user_email)',
      'CREATE INDEX IF NOT EXISTS idx_org_memberships_org ON org_memberships(organization_id)',
      "CREATE UNIQUE INDEX IF NOT EXISTS idx_org_one_owner ON org_memberships(organization_id) WHERE role = 'owner'",
    ],
  })

  // apps — App(.App Slug) entity type
  tables.push({
    tableName: 'apps',
    columns: [
      ...systemCols({ domain: false }),
      { name: 'slug', type: 'TEXT', extra: 'NOT NULL UNIQUE' },
      // "App has Name" → name TEXT
      { name: 'name', type: 'TEXT', extra: '' },
      // "App has App Type" → app_type TEXT
      { name: 'app_type', type: 'TEXT', extra: '' },
      // "App has Chat Endpoint" → chat_endpoint TEXT
      { name: 'chat_endpoint', type: 'TEXT', extra: '' },
      // "App belongs to Organization" → organization_id FK
      { name: 'organization_id', type: 'TEXT', extra: 'REFERENCES organizations(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_apps_slug ON apps(slug)',
      'CREATE INDEX IF NOT EXISTS idx_apps_org ON apps(organization_id)',
    ],
  })

  // domains — Domain(.Domain Slug) entity type
  tables.push({
    tableName: 'domains',
    columns: [
      // domains does not have a self-referential domain_id
      ...systemCols({ domain: false }),
      { name: 'domain_slug', type: 'TEXT', extra: 'NOT NULL UNIQUE' },
      // "Domain has Name" → name TEXT
      { name: 'name', type: 'TEXT', extra: '' },
      // "Domain belongs to App" → app_id FK
      { name: 'app_id', type: 'TEXT', extra: 'REFERENCES apps(id)' },
      // "Domain belongs to Organization" → organization_id FK
      { name: 'organization_id', type: 'TEXT', extra: 'REFERENCES organizations(id)' },
      // "Domain has Label" → label TEXT
      { name: 'label', type: 'TEXT', extra: '' },
      // "Domain has Access" + enum → access CHECK
      { name: 'access', type: 'TEXT', extra: "NOT NULL DEFAULT 'private' CHECK (access IN ('private', 'public'))" },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_domains_org ON domains(organization_id)',
      'CREATE INDEX IF NOT EXISTS idx_domains_slug ON domains(domain_slug)',
    ],
  })

  // ── Core Metamodel (from core.md) ───────────────────────────────────

  // nouns — Noun(.id) entity type
  tables.push({
    tableName: 'nouns',
    columns: [
      ...systemCols({ domain: true }),
      // "Noun has Name" → name column (from "Noun has Object Type", etc.)
      // Actually the Name reading is not for Noun itself in core.md, but
      // the nouns table needs a name column for the noun's own name.
      { name: 'name', type: 'TEXT', extra: 'NOT NULL' },
      // "Noun has Object Type" + enum values 'entity','value'
      { name: 'object_type', type: 'TEXT', extra: "NOT NULL DEFAULT 'entity' CHECK (object_type IN ('entity', 'value'))" },
      // "Noun is subtype of Noun" → self-referential FK
      { name: 'super_type_id', type: 'TEXT', extra: 'REFERENCES nouns(id)' },
      // "Noun has Plural"
      { name: 'plural', type: 'TEXT', extra: '' },
      // "Noun has Value Type Name"
      { name: 'value_type', type: 'TEXT', extra: '' },
      // "Noun has Format"
      { name: 'format', type: 'TEXT', extra: '' },
      // "Noun has Enum Values"
      { name: 'enum_values', type: 'TEXT', extra: '' },
      // "Noun has Minimum"
      { name: 'minimum', type: 'REAL', extra: '' },
      // "Noun has Maximum"
      { name: 'maximum', type: 'REAL', extra: '' },
      // "Noun has Pattern"
      { name: 'pattern', type: 'TEXT', extra: '' },
      // "Noun is described to AI by prompt Text"
      { name: 'prompt_text', type: 'TEXT', extra: '' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_nouns_domain ON nouns(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_nouns_name_domain ON nouns(name, domain_id)',
    ],
  })

  // graph_schemas — Graph Schema (subtype of Noun)
  tables.push({
    tableName: 'graph_schemas',
    columns: [
      ...systemCols({ domain: true }),
      // "Graph Schema has Title" (used as name in the DDL)
      { name: 'name', type: 'TEXT', extra: 'NOT NULL' },
      { name: 'title', type: 'TEXT', extra: '' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_graph_schemas_domain ON graph_schemas(domain_id)',
    ],
  })

  // readings — Reading(.id) entity type
  tables.push({
    tableName: 'readings',
    columns: [
      ...systemCols({ domain: true }),
      // "Reading has Text" + MC
      { name: 'text', type: 'TEXT', extra: 'NOT NULL' },
      // "Graph Schema has Reading" (UC on role 1: each Reading belongs to one Graph Schema)
      { name: 'graph_schema_id', type: 'TEXT', extra: 'REFERENCES graph_schemas(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_readings_domain ON readings(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_readings_schema ON readings(graph_schema_id)',
      'CREATE INDEX IF NOT EXISTS idx_readings_text_domain ON readings(text, domain_id)',
    ],
  })

  // roles — Role(.id) entity type
  tables.push({
    tableName: 'roles',
    columns: [
      ...systemCols({ domain: false }),
      // "Role is used in Reading" → reading_id FK
      { name: 'reading_id', type: 'TEXT', extra: 'REFERENCES readings(id)' },
      // "Noun plays Role" → noun_id FK
      { name: 'noun_id', type: 'TEXT', extra: 'REFERENCES nouns(id)' },
      // "Graph Schema has Role" → graph_schema_id FK
      { name: 'graph_schema_id', type: 'TEXT', extra: 'REFERENCES graph_schemas(id)' },
      // "Role has Position for Reading" → role_index (positional index)
      { name: 'role_index', type: 'INTEGER', extra: 'NOT NULL DEFAULT 0' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_roles_reading ON roles(reading_id)',
      'CREATE INDEX IF NOT EXISTS idx_roles_schema ON roles(graph_schema_id)',
    ],
  })

  // constraints — Constraint(.id) entity type
  tables.push({
    tableName: 'constraints',
    columns: [
      ...systemCols({ domain: true }),
      // "Constraint is of Constraint Type" → kind (code from Constraint Type entity)
      { name: 'kind', type: 'TEXT', extra: "NOT NULL CHECK (kind IN ('UC', 'MC', 'SS', 'XC', 'EQ', 'OR', 'XO', 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', 'AC', 'FC', 'VC'))" },
      // "Constraint has modality of Modality Type" → modality
      { name: 'modality', type: 'TEXT', extra: "NOT NULL DEFAULT 'Alethic' CHECK (modality IN ('Alethic', 'Deontic'))" },
      // Source text for round-tripping
      { name: 'text', type: 'TEXT', extra: '' },
      // "Set Comparison Constraint has Argument Length"
      { name: 'set_comparison_argument_length', type: 'INTEGER', extra: '' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_constraints_domain ON constraints(domain_id)',
    ],
  })

  // constraint_spans — objectification of "Constraint spans Role"
  tables.push({
    tableName: 'constraint_spans',
    columns: [
      ...systemCols({ domain: false }),
      // FK to constraint
      { name: 'constraint_id', type: 'TEXT', extra: 'NOT NULL REFERENCES constraints(id)' },
      // FK to role
      { name: 'role_id', type: 'TEXT', extra: 'NOT NULL REFERENCES roles(id)' },
      // "Constraint Span autofills from superset" → boolean flag
      { name: 'subset_autofill', type: 'INTEGER', extra: 'DEFAULT 0' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_constraint_spans_constraint ON constraint_spans(constraint_id)',
      'CREATE INDEX IF NOT EXISTS idx_constraint_spans_role ON constraint_spans(role_id)',
    ],
  })

  // ── State Machines (from state.md + core.md) ───────────────────────

  // state_machine_definitions — State Machine Definition(.Title within Domain)
  tables.push({
    tableName: 'state_machine_definitions',
    columns: [
      ...systemCols({ domain: true }),
      // Title (reference scheme)
      { name: 'title', type: 'TEXT', extra: '' },
      // "State Machine Definition is for Noun" → noun_id FK
      { name: 'noun_id', type: 'TEXT', extra: 'REFERENCES nouns(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_smd_domain ON state_machine_definitions(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_smd_noun ON state_machine_definitions(noun_id)',
    ],
  })

  // statuses — Status(.Name within State Machine Definition)
  tables.push({
    tableName: 'statuses',
    columns: [
      ...systemCols({ domain: false }),
      // Name (reference scheme)
      { name: 'name', type: 'TEXT', extra: 'NOT NULL' },
      // "Status belongs to State Machine Definition" → FK
      { name: 'state_machine_definition_id', type: 'TEXT', extra: 'NOT NULL REFERENCES state_machine_definitions(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_statuses_smd ON statuses(state_machine_definition_id)',
    ],
  })

  // event_types — Event Type(.id)
  tables.push({
    tableName: 'event_types',
    columns: [
      ...systemCols({ domain: true }),
      // "Event Type has Name" → name
      { name: 'name', type: 'TEXT', extra: 'NOT NULL' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_event_types_domain ON event_types(domain_id)',
    ],
  })

  // transitions — Transition(within State Machine Definition)
  tables.push({
    tableName: 'transitions',
    columns: [
      ...systemCols({ domain: false }),
      // "Transition has Status as source" → from_status_id FK
      { name: 'from_status_id', type: 'TEXT', extra: 'NOT NULL REFERENCES statuses(id)' },
      // "Transition has Status as target" → to_status_id FK
      { name: 'to_status_id', type: 'TEXT', extra: 'NOT NULL REFERENCES statuses(id)' },
      // "Transition is triggered by Event Type" → event_type_id FK
      { name: 'event_type_id', type: 'TEXT', extra: 'REFERENCES event_types(id)' },
      // "Verb is performed during Transition" → verb_id FK (reverse: verb table has transition_id)
      { name: 'verb_id', type: 'TEXT', extra: 'REFERENCES verbs(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_transitions_from ON transitions(from_status_id)',
      'CREATE INDEX IF NOT EXISTS idx_transitions_to ON transitions(to_status_id)',
    ],
  })

  // guards — Guard(.Name within Transition)
  tables.push({
    tableName: 'guards',
    columns: [
      ...systemCols({ domain: true }),
      // Name (reference scheme)
      { name: 'name', type: 'TEXT', extra: '' },
      // "Guard prevents Transition" → transition_id FK
      { name: 'transition_id', type: 'TEXT', extra: 'REFERENCES transitions(id)' },
      // "Guard references Graph Schema" → graph_schema_id FK
      { name: 'graph_schema_id', type: 'TEXT', extra: 'REFERENCES graph_schemas(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_guards_transition ON guards(transition_id)',
    ],
  })

  // verbs — Verb(.id) (from core.md)
  tables.push({
    tableName: 'verbs',
    columns: [
      ...systemCols({ domain: true }),
      // "Verb has Name" → name
      { name: 'name', type: 'TEXT', extra: 'NOT NULL' },
      // "Verb is performed in Status" → status_id FK
      { name: 'status_id', type: 'TEXT', extra: 'REFERENCES statuses(id)' },
      // "Verb is performed during Transition" → transition_id FK
      { name: 'transition_id', type: 'TEXT', extra: 'REFERENCES transitions(id)' },
      // "Graph is referenced by Verb" → graph_id FK
      { name: 'graph_id', type: 'TEXT', extra: 'REFERENCES graphs(id)' },
      // "Verb invokes Agent Definition" → agent_definition_id FK
      { name: 'agent_definition_id', type: 'TEXT', extra: 'REFERENCES agent_definitions(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_verbs_domain ON verbs(domain_id)',
    ],
  })

  // functions — Function(.id) (from core.md)
  tables.push({
    tableName: 'functions',
    columns: [
      ...systemCols({ domain: true }),
      // "Function has Name"
      { name: 'name', type: 'TEXT', extra: '' },
      // "Function has callback URI" → callback_url
      { name: 'callback_url', type: 'TEXT', extra: '' },
      // "Function has HTTP Method" → http_method
      { name: 'http_method', type: 'TEXT', extra: "DEFAULT 'POST'" },
      // "Function has Header" → headers (stored as JSON)
      { name: 'headers', type: 'TEXT', extra: '' },
      // "Verb executes Function" (reverse FK: function belongs to verb)
      { name: 'verb_id', type: 'TEXT', extra: 'REFERENCES verbs(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_functions_verb ON functions(verb_id)',
    ],
  })

  // streams — Stream(.id) (from core.md)
  tables.push({
    tableName: 'streams',
    columns: [
      ...systemCols({ domain: true }),
      // "Stream has Name"
      { name: 'name', type: 'TEXT', extra: 'NOT NULL' },
    ],
    tableConstraints: [],
    indexes: [],
  })

  // ── AI Agents (from agents.md) ─────────────────────────────────────

  // models — Model(.code)
  tables.push({
    tableName: 'models',
    columns: [
      ...systemCols({ domain: false }),
      // "Model has Name"
      { name: 'name', type: 'TEXT', extra: '' },
      // .code reference scheme
      { name: 'code', type: 'TEXT', extra: '' },
    ],
    tableConstraints: [],
    indexes: [],
  })

  // agent_definitions — Agent Definition(.id)
  tables.push({
    tableName: 'agent_definitions',
    columns: [
      ...systemCols({ domain: true }),
      // "Agent Definition has Name"
      { name: 'name', type: 'TEXT', extra: 'NOT NULL' },
      // "Agent Definition uses Model" → model_id FK
      { name: 'model_id', type: 'TEXT', extra: 'REFERENCES models(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_agent_definitions_domain ON agent_definitions(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_agent_definitions_model ON agent_definitions(model_id)',
    ],
  })

  // agents — Agent(.id)
  tables.push({
    tableName: 'agents',
    columns: [
      ...systemCols({ domain: true }),
      // "Agent is instance of Agent Definition" → agent_definition_id FK
      { name: 'agent_definition_id', type: 'TEXT', extra: 'NOT NULL REFERENCES agent_definitions(id)' },
      // "Agent is for Resource" → resource_id FK
      { name: 'resource_id', type: 'TEXT', extra: 'REFERENCES resources(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_agents_definition ON agents(agent_definition_id)',
      'CREATE INDEX IF NOT EXISTS idx_agents_domain ON agents(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_agents_resource ON agents(resource_id)',
    ],
  })

  // completions — Completion(.id)
  tables.push({
    tableName: 'completions',
    columns: [
      ...systemCols({ domain: true }),
      // "Completion belongs to Agent" → agent_id FK
      { name: 'agent_id', type: 'TEXT', extra: 'NOT NULL REFERENCES agents(id)' },
      // "Completion has input Text" → input_text
      { name: 'input_text', type: 'TEXT', extra: '' },
      // "Completion has output Text" → output_text
      { name: 'output_text', type: 'TEXT', extra: '' },
      // "Completion occurred at Timestamp" → occurred_at
      { name: 'occurred_at', type: 'TEXT', extra: "NOT NULL DEFAULT (datetime('now'))" },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_completions_agent ON completions(agent_id)',
      'CREATE INDEX IF NOT EXISTS idx_completions_domain ON completions(domain_id)',
    ],
  })

  // ── Runtime Instances (from instances.md) ──────────────────────────

  // citations — Citation
  tables.push({
    tableName: 'citations',
    columns: [
      ...systemCols({ domain: true }),
      // "Citation has Text" → text
      { name: 'text', type: 'TEXT', extra: 'NOT NULL' },
      // "Citation has URI" → uri
      { name: 'uri', type: 'TEXT', extra: '' },
      // "Citation has Retrieval Date" → retrieval_date
      { name: 'retrieval_date', type: 'TEXT', extra: '' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_citations_domain ON citations(domain_id)',
    ],
  })

  // graphs — Graph (subtype of Resource)
  tables.push({
    tableName: 'graphs',
    columns: [
      ...systemCols({ domain: true }),
      // "Graph is of Graph Schema" → graph_schema_id FK
      { name: 'graph_schema_id', type: 'TEXT', extra: 'REFERENCES graph_schemas(id)' },
      // "Graph is completed" → is_done boolean
      { name: 'is_done', type: 'INTEGER', extra: 'NOT NULL DEFAULT 0' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_graphs_domain ON graphs(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_graphs_schema ON graphs(graph_schema_id)',
    ],
  })

  // graph_citations — junction for "Graph cites Citation"
  tables.push({
    tableName: 'graph_citations',
    columns: [
      ...systemCols({ domain: true }),
      // FK to Graph
      { name: 'graph_id', type: 'TEXT', extra: 'NOT NULL REFERENCES graphs(id)' },
      // FK to Citation
      { name: 'citation_id', type: 'TEXT', extra: 'NOT NULL REFERENCES citations(id)' },
    ],
    tableConstraints: ['UNIQUE(graph_id, citation_id)'],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_graph_citations_graph ON graph_citations(graph_id)',
      'CREATE INDEX IF NOT EXISTS idx_graph_citations_citation ON graph_citations(citation_id)',
    ],
  })

  // resources — Resource(.Reference within Domain)
  tables.push({
    tableName: 'resources',
    columns: [
      ...systemCols({ domain: true }),
      // "Resource is instance of Noun" → noun_id FK
      { name: 'noun_id', type: 'TEXT', extra: 'REFERENCES nouns(id)' },
      // "Resource has Reference"
      { name: 'reference', type: 'TEXT', extra: '' },
      // "Resource has Value"
      { name: 'value', type: 'TEXT', extra: '' },
      // "Resource is created by User" → created_by
      { name: 'created_by', type: 'TEXT', extra: '' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_resources_domain ON resources(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_resources_noun ON resources(noun_id)',
    ],
  })

  // resource_roles — "Graph uses Resource for Role" junction
  tables.push({
    tableName: 'resource_roles',
    columns: [
      ...systemCols({ domain: true }),
      // FK to Graph
      { name: 'graph_id', type: 'TEXT', extra: 'NOT NULL REFERENCES graphs(id)' },
      // FK to Resource
      { name: 'resource_id', type: 'TEXT', extra: 'NOT NULL REFERENCES resources(id)' },
      // FK to Role
      { name: 'role_id', type: 'TEXT', extra: 'NOT NULL REFERENCES roles(id)' },
    ],
    tableConstraints: ['UNIQUE(graph_id, role_id)'],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_resource_roles_graph ON resource_roles(graph_id)',
    ],
  })

  // state_machines — State Machine(.Name within Domain)
  tables.push({
    tableName: 'state_machines',
    columns: [
      ...systemCols({ domain: true }),
      // Name
      { name: 'name', type: 'TEXT', extra: '' },
      // "State Machine is instance of State Machine Definition" → FK
      { name: 'state_machine_definition_id', type: 'TEXT', extra: 'REFERENCES state_machine_definitions(id)' },
      // "State Machine is currently in Status" → current_status_id FK
      { name: 'current_status_id', type: 'TEXT', extra: 'REFERENCES statuses(id)' },
      // "State Machine is for Resource" → resource_id FK
      { name: 'resource_id', type: 'TEXT', extra: 'REFERENCES resources(id)' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_state_machines_domain ON state_machines(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_state_machines_definition ON state_machines(state_machine_definition_id)',
    ],
  })

  // events — Event(.id)
  tables.push({
    tableName: 'events',
    columns: [
      ...systemCols({ domain: true }),
      // "Event is of Event Type" → event_type_id FK
      { name: 'event_type_id', type: 'TEXT', extra: 'REFERENCES event_types(id)' },
      // Event implicitly belongs to a state machine (from Event Triggered Transition In State Machine)
      { name: 'state_machine_id', type: 'TEXT', extra: 'REFERENCES state_machines(id)' },
      // Event can reference a Graph
      { name: 'graph_id', type: 'TEXT', extra: 'REFERENCES graphs(id)' },
      // "Event has Data" → data
      { name: 'data', type: 'TEXT', extra: '' },
      // "Event occurred at Timestamp" → occurred_at
      { name: 'occurred_at', type: 'TEXT', extra: "NOT NULL DEFAULT (datetime('now'))" },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_events_state_machine ON events(state_machine_id)',
      'CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type_id)',
    ],
  })

  // guard_runs — Guard Run(.Name within Event)
  tables.push({
    tableName: 'guard_runs',
    columns: [
      ...systemCols({ domain: true }),
      // Name
      { name: 'name', type: 'TEXT', extra: '' },
      // "Guard Run is for Guard" → guard_id FK
      { name: 'guard_id', type: 'TEXT', extra: 'REFERENCES guards(id)' },
      // "Guard Run references Graph" → graph_id FK
      { name: 'graph_id', type: 'TEXT', extra: 'REFERENCES graphs(id)' },
      // "Guard Run has Result" → result
      { name: 'result', type: 'INTEGER', extra: '' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_guard_runs_guard ON guard_runs(guard_id)',
    ],
  })

  // generators — not from readings, but an operational table for generator output
  tables.push({
    tableName: 'generators',
    columns: [
      ...systemCols({ domain: true }),
      { name: 'output_format', type: 'TEXT', extra: "NOT NULL DEFAULT 'openapi'" },
      { name: 'title', type: 'TEXT', extra: '' },
      { name: 'version', type: 'TEXT', extra: "DEFAULT '1.0'" },
      { name: 'output', type: 'TEXT', extra: '' },
    ],
    tableConstraints: [],
    indexes: [
      'CREATE INDEX IF NOT EXISTS idx_generators_domain ON generators(domain_id)',
      'CREATE INDEX IF NOT EXISTS idx_generators_format ON generators(output_format)',
    ],
  })

  return tables
}

// ── DDL rendering ───────────────────────────────────────────────────────────

function renderTable(spec: TableSpec): string[] {
  const ddl: string[] = []

  // Column definitions
  const colDefs = spec.columns.map(c => {
    const parts = [c.name, c.type]
    if (c.extra) parts.push(c.extra)
    return parts.join(' ')
  })

  // Table constraints (UNIQUE, etc.)
  const allDefs = [...colDefs, ...spec.tableConstraints]

  ddl.push(`CREATE TABLE IF NOT EXISTS ${spec.tableName} (\n  ${allDefs.join(',\n  ')}\n)`)

  // Indexes
  for (const idx of spec.indexes) {
    ddl.push(idx)
  }

  return ddl
}

// ── Main Script ─────────────────────────────────────────────────────────────

function main() {
  const rootDir = path.resolve(import.meta.dirname!, '..')
  const readingsDir = path.join(rootDir, 'readings')
  const outFile = path.join(rootDir, 'src', 'schema', 'bootstrap.ts')

  // Read and parse all readings files in dependency order
  const readingsFiles = [
    'core.md',
    'organizations.md',
    'state.md',
    'instances.md',
    'agents.md',
  ]

  const allNouns = new Map<string, NounInfo>()
  const allReadings: ReadingInfo[] = []
  const allConstraints: ConstraintInfo[] = []
  const allSubtypes: Array<{ child: string; parent: string }> = []

  for (const file of readingsFiles) {
    const filePath = path.join(readingsDir, file)
    const text = fs.readFileSync(filePath, 'utf-8')

    const existingNouns = [...allNouns.values()].map(n => ({
      name: n.name,
      id: '',
      objectType: n.objectType,
    }))

    const result = parseFORML2(text, existingNouns)

    for (const n of result.nouns) {
      if (!allNouns.has(n.name)) {
        allNouns.set(n.name, {
          name: n.name,
          objectType: n.objectType,
          enumValues: n.enumValues,
        })
      }
    }

    for (const r of result.readings) {
      allReadings.push({ text: r.text, nouns: r.nouns, predicate: r.predicate })
    }

    for (const c of result.constraints) {
      allConstraints.push({
        kind: c.kind,
        modality: c.modality,
        reading: c.reading,
        roles: c.roles,
        text: c.text,
      })
    }

    if (result.subtypes) {
      allSubtypes.push(...result.subtypes)
    }
  }

  // Debug output
  console.log(`Parsed ${readingsFiles.length} readings files`)
  console.log(`  Nouns: ${allNouns.size} (${[...allNouns.values()].filter(n => n.objectType === 'entity').length} entity, ${[...allNouns.values()].filter(n => n.objectType === 'value').length} value)`)
  console.log(`  Readings: ${allReadings.length}`)
  console.log(`  Constraints: ${allConstraints.length}`)
  console.log(`  Subtypes: ${allSubtypes.length}`)

  // Build table definitions
  const tables = buildTables(allNouns, allReadings, allConstraints, allSubtypes)

  // Render DDL
  const allDdl: string[] = []
  for (const table of tables) {
    allDdl.push(...renderTable(table))
  }

  // Generate output file
  // The generators table uses 'version' both as a system column and a business column.
  // In the DDL the system version_num column replaces the default version column.
  // Fix: generators needs version_num instead of version for optimistic concurrency.
  const processedDdl = allDdl.map(stmt => {
    // generators: rename the second version column to avoid conflict
    if (stmt.includes('CREATE TABLE IF NOT EXISTS generators')) {
      // The system column 'version INTEGER NOT NULL DEFAULT 1' comes before
      // the business column 'version TEXT DEFAULT ...'.
      // Replace system 'version' with 'version_num' in generators table.
      stmt = stmt.replace(
        "version INTEGER NOT NULL DEFAULT 1",
        "version_num INTEGER NOT NULL DEFAULT 1"
      )
    }
    return stmt
  })

  const fileContent = `/**
 * Bootstrap DDL for GraphDL metamodel tables.
 *
 * AUTO-GENERATED from readings/*.md — do not edit manually.
 * Regenerate with: npx tsx scripts/generate-bootstrap.ts
 *
 * Source files:
${readingsFiles.map(f => ` *   readings/${f}`).join('\n')}
 */

export const BOOTSTRAP_DDL: string[] = [
${processedDdl.map(stmt => '  `' + stmt + '`').join(',\n\n')},
]
`

  fs.writeFileSync(outFile, fileContent, 'utf-8')

  // Summary
  const tableCount = processedDdl.filter(s => s.startsWith('CREATE TABLE')).length
  const indexCount = processedDdl.filter(s => s.includes('CREATE INDEX') || s.includes('CREATE UNIQUE INDEX')).length
  console.log(`\nGenerated ${outFile}`)
  console.log(`  ${processedDdl.length} DDL statements (${tableCount} tables, ${indexCount} indexes)`)

  // Verify all expected tables are present
  const expectedTables = [
    'organizations', 'org_memberships', 'apps', 'domains',
    'nouns', 'graph_schemas', 'readings', 'roles', 'constraints', 'constraint_spans',
    'state_machine_definitions', 'statuses', 'event_types', 'transitions', 'guards',
    'verbs', 'functions', 'streams',
    'citations', 'graphs', 'graph_citations', 'resources', 'resource_roles',
    'state_machines', 'events', 'guard_runs',
    'generators', 'models', 'agent_definitions', 'agents', 'completions',
  ]

  const generatedTables = processedDdl
    .filter(s => s.startsWith('CREATE TABLE'))
    .map(s => s.match(/CREATE TABLE IF NOT EXISTS (\w+)/)?.[1])
    .filter(Boolean) as string[]

  const missing = expectedTables.filter(t => !generatedTables.includes(t))
  if (missing.length > 0) {
    console.error(`\nERROR: Missing tables: ${missing.join(', ')}`)
    process.exit(1)
  }

  // Verify CHECK constraints are present
  const fullDdl = processedDdl.join('\n')
  const expectedChecks = [
    "object_type IN ('entity', 'value')",
    "kind IN ('UC', 'MC', 'SS', 'XC', 'EQ', 'OR', 'XO', 'IR', 'AS', 'AT', 'SY', 'IT', 'TR', 'AC', 'FC', 'VC')",
    "modality IN ('Alethic', 'Deontic')",
    "visibility IN ('private', 'public')",
    "role IN ('owner', 'admin', 'member')",
  ]
  for (const check of expectedChecks) {
    if (!fullDdl.includes(check)) {
      console.error(`\nERROR: Missing CHECK constraint: ${check}`)
      process.exit(1)
    }
  }

  console.log('\nAll expected tables and CHECK constraints present.')
}

main()
