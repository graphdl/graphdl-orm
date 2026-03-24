/**
 * Maps noun names (as they appear in readings) to their metamodel table names.
 * Used by queryEntities to resolve the correct table for metamodel entities.
 */
export const NOUN_TABLE_MAP: Record<string, string> = {
  'Organization': 'organizations',
  'Domain': 'domains',
  'App': 'apps',
  'Noun': 'nouns',
  'Graph Schema': 'graph_schemas',
  'Reading': 'readings',
  'Role': 'roles',
  'Constraint': 'constraints',
  'Constraint Span': 'constraint_spans',
  'State Machine Definition': 'state_machine_definitions',
  'Status': 'statuses',
  'Transition': 'transitions',
  'Guard': 'guards',
  'Event Type': 'event_types',
  'Verb': 'verbs',
  'Function': 'functions',
  'Stream': 'streams',
  'Resource': 'resources',
  'Graph': 'graphs',
  'State Machine': 'state_machines',
  'Event': 'events',
  'Guard Run': 'guard_runs',
  'Agent Definition': 'agent_definitions',
  'Agent': 'agents',
  'Model': 'models',
  'Completion': 'completions',
  'Citation': 'citations',
}

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

  // Citations
  'citations': 'citations',
  'graph-citations': 'graph_citations',

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
  'citations', 'graph-citations',
  'state-machines', 'events', 'guard-runs',
  'agents', 'completions',
])

