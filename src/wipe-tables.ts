/**
 * All tables to delete during wipeAllData(), in leaf-to-root dependency order.
 * Includes every bootstrap table plus infrastructure tables (cdc_events).
 *
 * Extracted to its own module so it can be imported in tests without pulling
 * in the Cloudflare DurableObject runtime dependency from do.ts.
 */
export const WIPE_TABLES: readonly string[] = [
  // infrastructure
  'cdc_events',
  // leaf instances
  'generators', 'guard_runs', 'events', 'state_machines',
  'resource_roles', 'graph_citations', 'resources', 'graphs',
  'completions', 'agents', 'agent_definitions', 'models',
  'citations',
  'functions', 'streams', 'verbs',
  'guards', 'transitions', 'statuses', 'event_types', 'state_machine_definitions',
  'constraint_spans', 'constraints', 'roles', 'readings', 'graph_schemas',
  'nouns',
  'domains', 'apps', 'org_memberships', 'organizations',
] as const
