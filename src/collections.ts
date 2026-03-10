/**
 * Maps Payload CMS collection slugs (kebab-case) to SQLite table names (snake_case).
 *
 * The apis worker rawProxy uses Payload slugs in /graphdl/raw/<slug> URLs.
 * This map translates them to our 3NF table names.
 */
export const COLLECTION_TABLE_MAP: Record<string, string> = {
  // Organizations & access
  'organizations': 'organizations',
  'org-memberships': 'org_memberships',
  'apps': 'apps',
  'domains': 'domains',

  // Core metamodel
  'nouns': 'nouns',
  'graph-schemas': 'graph_schemas',
  'readings': 'readings',
  'roles': 'roles',
  'constraints': 'constraints',
  'constraint-spans': 'constraint_spans',

  // State machine definitions
  'state-machine-definitions': 'state_machine_definitions',
  'statuses': 'statuses',
  'transitions': 'transitions',
  'guards': 'guards',
  'event-types': 'event_types',
  'verbs': 'verbs',
  'functions': 'functions',
  'streams': 'streams',

  // AI agent entities
  'models': 'models',
  'agent-definitions': 'agent_definitions',
  'agents': 'agents',
  'completions': 'completions',

  // Generator output
  'generators': 'generators',

  // Runtime instances
  'graphs': 'graphs',
  'resources': 'resources',
  'resource-roles': 'resource_roles',
  'state-machines': 'state_machines',
  'events': 'events',
  'guard-runs': 'guard_runs',
}

/** All supported Payload collection slugs. */
export const COLLECTION_SLUGS = Object.keys(COLLECTION_TABLE_MAP)

/** Instance collections — scoped per-domain. */
export const INSTANCE_COLLECTIONS = new Set([
  'graphs', 'resources', 'resource-roles',
  'state-machines', 'events', 'guard-runs',
  'agents', 'completions',
])

/**
 * Column mapping per table. Maps Payload field names to SQLite column names.
 * Only fields that differ from identity mapping need entries.
 */
export const FIELD_MAP: Record<string, Record<string, string>> = {
  nouns: { domain: 'domain_id', superType: 'super_type_id', objectType: 'object_type', promptText: 'prompt_text', enumValues: 'enum_values', valueType: 'value_type' },
  graph_schemas: { domain: 'domain_id' },
  readings: { domain: 'domain_id', graphSchema: 'graph_schema_id' },
  roles: { reading: 'reading_id', noun: 'noun_id', graphSchema: 'graph_schema_id', roleIndex: 'role_index' },
  constraints: { domain: 'domain_id' },
  constraint_spans: { constraint: 'constraint_id', role: 'role_id' },
  apps: { organization: 'organization_id' },
  domains: { domainSlug: 'domain_slug', organization: 'organization_id', app: 'app_id' },
  organizations: {},
  org_memberships: { organization: 'organization_id', userEmail: 'user_email' },
  state_machine_definitions: { domain: 'domain_id', noun: 'noun_id' },
  statuses: { stateMachineDefinition: 'state_machine_definition_id', domain: 'domain_id' },
  transitions: { from: 'from_status_id', to: 'to_status_id', eventType: 'event_type_id', verb: 'verb_id', domain: 'domain_id' },
  guards: { transition: 'transition_id', graphSchema: 'graph_schema_id', domain: 'domain_id' },
  event_types: { domain: 'domain_id' },
  verbs: { status: 'status_id', transition: 'transition_id', graph: 'graph_id', agentDefinition: 'agent_definition_id', domain: 'domain_id' },
  functions: { callbackUrl: 'callback_url', httpMethod: 'http_method', headers: 'headers', verb: 'verb_id', domain: 'domain_id' },
  streams: { domain: 'domain_id' },
  models: {},
  agent_definitions: { model: 'model_id', domain: 'domain_id' },
  agents: { agentDefinition: 'agent_definition_id', resource: 'resource_id', domain: 'domain_id' },
  completions: { agent: 'agent_id', inputText: 'input_text', outputText: 'output_text', occurredAt: 'occurred_at', domain: 'domain_id' },
  graphs: { graphSchema: 'graph_schema_id', domain: 'domain_id', isDone: 'is_done' },
  resources: { noun: 'noun_id', domain: 'domain_id' },
  resource_roles: { graph: 'graph_id', resource: 'resource_id', role: 'role_id', domain: 'domain_id' },
  state_machines: { stateMachineDefinition: 'state_machine_definition_id', stateMachineType: 'state_machine_definition_id', currentStatus: 'current_status_id', stateMachineStatus: 'current_status_id', resource: 'resource_id', domain: 'domain_id' },
  events: { eventType: 'event_type_id', stateMachine: 'state_machine_id', graph: 'graph_id', occurredAt: 'occurred_at', domain: 'domain_id' },
  generators: { domain: 'domain_id', outputFormat: 'output_format', versionNum: 'version_num' },
  guard_runs: { guard: 'guard_id', graph: 'graph_id', domain: 'domain_id' },
}

/**
 * Maps FK column names to their target table.
 * Derived from REFERENCES clauses in DDL.
 * Used by buildWhereClause to resolve dot-notation queries
 * like `where[domain.domainSlug][equals]=joey`.
 */
export const FK_TARGET_TABLE: Record<string, string> = {
  app_id: 'apps',
  domain_id: 'domains',
  organization_id: 'organizations',
  super_type_id: 'nouns',
  graph_schema_id: 'graph_schemas',
  reading_id: 'readings',
  noun_id: 'nouns',
  constraint_id: 'constraints',
  role_id: 'roles',
  state_machine_definition_id: 'state_machine_definitions',
  from_status_id: 'statuses',
  to_status_id: 'statuses',
  event_type_id: 'event_types',
  verb_id: 'verbs',
  transition_id: 'transitions',
  guard_id: 'guards',
  graph_id: 'graphs',
  resource_id: 'resources',
  state_machine_id: 'state_machines',
  current_status_id: 'statuses',
  model_id: 'models',
  agent_definition_id: 'agent_definitions',
  agent_id: 'agents',
}

/**
 * Reverse (has-many) relationships for depth population.
 * Maps: parent table → { payloadField → { childTable, fkColumn } }
 * Used to populate arrays like app.domains when depth>0.
 */
export const REVERSE_FK_MAP: Record<string, Record<string, { childCollection: string; fkColumn: string }>> = {
  apps: {
    domains: { childCollection: 'domains', fkColumn: 'app_id' },
  },
}
