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
  domains: { domainSlug: 'domain_slug', organization: 'organization_id' },
  organizations: {},
  org_memberships: { organization: 'organization_id', userEmail: 'user_email' },
  state_machine_definitions: { domain: 'domain_id', noun: 'noun_id' },
  statuses: { stateMachineDefinition: 'state_machine_definition_id' },
  transitions: { from: 'from_status_id', to: 'to_status_id', eventType: 'event_type_id', verb: 'verb_id' },
  guards: { transition: 'transition_id', graphSchema: 'graph_schema_id', domain: 'domain_id' },
  event_types: { domain: 'domain_id' },
  verbs: { status: 'status_id', transition: 'transition_id', graph: 'graph_id', domain: 'domain_id' },
  functions: { callbackUrl: 'callback_url', httpMethod: 'http_method', verb: 'verb_id', domain: 'domain_id' },
  streams: { domain: 'domain_id' },
  graphs: { graphSchema: 'graph_schema_id', domain: 'domain_id', isDone: 'is_done' },
  resources: { noun: 'noun_id', domain: 'domain_id' },
  resource_roles: { graph: 'graph_id', resource: 'resource_id', role: 'role_id', domain: 'domain_id' },
  state_machines: { stateMachineDefinition: 'state_machine_definition_id', stateMachineType: 'state_machine_definition_id', currentStatus: 'current_status_id', stateMachineStatus: 'current_status_id', resource: 'resource_id', domain: 'domain_id' },
  events: { eventType: 'event_type_id', stateMachine: 'state_machine_id', graph: 'graph_id', occurredAt: 'occurred_at' },
  guard_runs: { guard: 'guard_id', graph: 'graph_id', domain: 'domain_id' },
}
