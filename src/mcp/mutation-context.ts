export const MUTATION_CONTEXT_VERSION = 'arest-context:v1'

export const MUTATING_CONTEXT_TOOLS = ['apply', 'compile', 'propose'] as const

export type MutationContextTool = typeof MUTATING_CONTEXT_TOOLS[number] | string
export type MutationContextDetail = 'summary' | 'full'

export interface MutationPromptSource {
  name: string
  description: string
  text?: string
}

export interface MutationContextScope {
  app?: string
  db?: string
  readingsDir?: string
}

export interface MutationContext {
  version: string
  receipt: string
  receipt_field: 'context_receipt'
  receipt_applies_to: readonly string[]
  scope?: MutationContextScope
  rules: readonly string[]
  anti_patterns: readonly string[]
  how_to: readonly string[]
  prompt_bundle: Array<{
    name: string
    description: string
    digest: string
    bytes: number | null
    text?: string
  }>
}

export const CONTEXT_RECEIPT_FIELD_DESCRIPTION =
  'Required for mutating fact operations. Call the context tool first, read the modeling rules/prompts it returns, then pass its receipt here.'

export const MUTATION_CONTEXT_DESCRIPTION =
  'Load AREST modeling context and receive the context_receipt required by mutating tools. Use this before apply, compile, or propose.'

export const MUTATION_TOOL_DESCRIPTION =
  'Mutation gate: call context first and pass its receipt as context_receipt. The gate exists so fact writes happen with the AREST modeling prompts in context.'

export const DEFAULT_MUTATION_PROMPTS: readonly MutationPromptSource[] = [
  { name: 'overview', description: 'AREST overview, CSDP flow, and FORML2 document structure' },
  { name: 'design-principles', description: 'Facts all the way down, readings as source of truth, and self-modification principles' },
  { name: 'entity-modeling', description: 'Entity/value types, reference schemes, UCs, multiplicity, and objectification' },
  { name: 'verbalization', description: 'ORM2 verbalization patterns and constraint wording' },
  { name: 'advanced-constraints', description: 'Subtype partitions, subset constraints, and ring constraints' },
  { name: 'derivation-deontic', description: 'Derivation rules and deontic/alethic modality' },
  { name: 'api', description: 'AREST CLI, MCP, and HTTP API reference' },
]

export const MUTATION_CONTEXT_RULES: readonly string[] = [
  'The selected AREST app and active fact storage medium are the implicit Universe of Discourse: do not create a UoD meta-fact just to scope ordinary facts.',
  'Represent knowledge as typed FORML entity types, value types, fact types, constraints, and instance facts.',
  'Prefer atomic fact types over prose blobs. If a value needs text, model the role and value type explicitly.',
  'Use nouns with reference schemes for independent identity. Use value types for identifying values and scalars.',
  'Declare fact types in natural-language readings and let the compiler derive normalized storage.',
  'Objectify a fact type only when the objectified relationship has a spanning uniqueness constraint or its own reference scheme.',
  'Define workflows as state-machine facts: noun binding, statuses, initial/terminal status, transitions, events, from/to roles.',
  'Use query/schema/actions/tutor first when uncertain; write only after the model shape is known.',
  'Use propose for governed schema evolution and compile only when immediate self-modification is intentional.',
]

export const MUTATION_CONTEXT_ANTI_PATTERNS: readonly string[] = [
  'Do not create catch-all facts such as Fact Note, Note Text, Decision Text, User Text, or Prose Blob for agent memory.',
  'Do not encode domain structure in generic Summary, Description, Rationale, or Text fields unless those are explicit domain value types.',
  'Do not invent shorthand syntax. Use the FORML2 forms documented by the tutor and prompt bundle.',
  'Do not denormalize relationships into fields when the relationship is itself a fact type.',
  'Do not bypass state machines by directly setting workflow status unless that direct fact is explicitly modeled.',
]

export const MUTATION_CONTEXT_HOW_TO: readonly string[] = [
  'Call schema, tutor, or query to inspect the existing model.',
  'Call context and read the returned rules and prompt manifest.',
  'For population changes, call apply with operation=create/update/transition and the context_receipt.',
  'For schema changes, write FORML2 readings and call compile or propose with the context_receipt.',
  'If a mutation is rejected, repair the model or use propose to record the schema change workflow.',
]

function fnv1a64(text: string): string {
  let hash = 0xcbf29ce484222325n
  const prime = 0x100000001b3n
  const mask = 0xffffffffffffffffn
  for (let i = 0; i < text.length; i++) {
    hash ^= BigInt(text.charCodeAt(i))
    hash = (hash * prime) & mask
  }
  return hash.toString(16).padStart(16, '0')
}

export function digestText(text: string): string {
  return `fnv1a64:${fnv1a64(text)}`
}

export function buildMutationContext(options: {
  detail?: MutationContextDetail
  prompts?: readonly MutationPromptSource[]
  scope?: MutationContextScope
} = {}): MutationContext {
  const detail = options.detail ?? 'summary'
  const promptSources = options.prompts ?? DEFAULT_MUTATION_PROMPTS
  const promptBundle = promptSources.map((source) => {
    const text = source.text
    return {
      name: source.name,
      description: source.description,
      digest: digestText(text ?? `${source.name}\n${source.description}`),
      bytes: text === undefined ? null : text.length,
      ...(detail === 'full' && text !== undefined ? { text } : {}),
    }
  })
  const receiptSeed = JSON.stringify({
    version: MUTATION_CONTEXT_VERSION,
    prompts: promptBundle.map(({ name, digest }) => ({ name, digest })),
    scope: options.scope ?? null,
    rules: MUTATION_CONTEXT_RULES,
    anti_patterns: MUTATION_CONTEXT_ANTI_PATTERNS,
    how_to: MUTATION_CONTEXT_HOW_TO,
  })
  return {
    version: MUTATION_CONTEXT_VERSION,
    receipt: `${MUTATION_CONTEXT_VERSION}:${digestText(receiptSeed)}`,
    receipt_field: 'context_receipt',
    receipt_applies_to: MUTATING_CONTEXT_TOOLS,
    ...(options.scope ? { scope: options.scope } : {}),
    rules: MUTATION_CONTEXT_RULES,
    anti_patterns: MUTATION_CONTEXT_ANTI_PATTERNS,
    how_to: MUTATION_CONTEXT_HOW_TO,
    prompt_bundle: promptBundle,
  }
}

export function mutationReceiptError(
  tool: MutationContextTool,
  receivedReceipt: string | undefined,
  context: MutationContext,
) {
  return {
    error: receivedReceipt ? 'context_receipt_stale' : 'context_receipt_required',
    tool,
    message: `Call the context tool first, read the AREST modeling rules/prompts, then retry ${tool} with context_receipt set to the returned receipt.`,
    expected_context_version: context.version,
    received_receipt: receivedReceipt ?? null,
    receipt_field: context.receipt_field,
    required_context: {
      version: context.version,
      scope: context.scope ?? null,
      rules: context.rules,
      anti_patterns: context.anti_patterns,
      how_to: context.how_to,
      prompt_bundle: context.prompt_bundle.map(({ name, description, digest, bytes }) => ({
        name,
        description,
        digest,
        bytes,
      })),
    },
  }
}

const PROSE_BLOB_NAMES = new Set([
  'fact note',
  'note text',
  'decision text',
  'user text',
  'prose blob',
  'free form text',
  'free-form text',
])

function suspiciousFieldNames(fields: unknown): string[] {
  if (!fields || typeof fields !== 'object' || Array.isArray(fields)) return []
  return Object.keys(fields as Record<string, unknown>)
    .filter((name) => PROSE_BLOB_NAMES.has(name.trim().toLowerCase()))
}

function suspiciousReadingTerms(readings: string): string[] {
  const found: string[] = []
  for (const term of PROSE_BLOB_NAMES) {
    const pattern = new RegExp(`\\b${term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`, 'i')
    if (pattern.test(readings)) found.push(term)
  }
  return found
}

export function mutationModelingViolations(tool: MutationContextTool, payload: Record<string, unknown>): string[] {
  const violations: string[] = []
  if (tool === 'compile') {
    const readings = typeof payload.readings === 'string' ? payload.readings : ''
    if (!readings.trim()) violations.push('compile requires non-empty FORML2 readings')
    const terms = suspiciousReadingTerms(readings)
    if (terms.length) {
      violations.push(`readings contain catch-all prose-memory terms: ${terms.join(', ')}`)
    }
  }
  if (tool === 'propose') {
    const readings = Array.isArray(payload.readings)
      ? payload.readings.filter((r): r is string => typeof r === 'string').join('\n')
      : ''
    const terms = suspiciousReadingTerms(readings)
    if (terms.length) {
      violations.push(`proposed readings contain catch-all prose-memory terms: ${terms.join(', ')}`)
    }
  }
  if (tool === 'apply') {
    const operation = String(payload.operation ?? '')
    if ((operation === 'create' || operation === 'update') && suspiciousFieldNames(payload.fields).length) {
      violations.push(`fields contain catch-all prose-memory names: ${suspiciousFieldNames(payload.fields).join(', ')}`)
    }
    if (operation === 'transition' && (!payload.id || !payload.event)) {
      violations.push('transition requires id and event')
    }
  }
  return violations
}

export function enforceMutationContext(options: {
  tool: MutationContextTool
  receivedReceipt?: string
  context: MutationContext
  payload?: Record<string, unknown>
}): { ok: true } | { ok: false; error: Record<string, unknown> } {
  if (options.receivedReceipt !== options.context.receipt) {
    return {
      ok: false,
      error: mutationReceiptError(options.tool, options.receivedReceipt, options.context),
    }
  }
  const violations = mutationModelingViolations(options.tool, options.payload ?? {})
  if (violations.length) {
    return {
      ok: false,
      error: {
        error: 'modeling_context_violation',
        tool: options.tool,
        message: 'The mutation conflicts with AREST modeling guardrails from the context prompt bundle.',
        violations,
        next_step: 'Inspect schema/tutor/context, then remodel the fact types or use propose with typed FORML2 readings.',
      },
    }
  }
  return { ok: true }
}
