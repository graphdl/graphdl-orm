import { tokenizeReading } from '../claims/tokenize'
import { ensure, createWithHook, refreshNouns, type HookResult, EMPTY_RESULT, type HookContext } from './index'

/**
 * Reading afterCreate hook.
 *
 * 1. Split text into fact type line + indented constraint lines
 * 2. Tokenize reading against known nouns
 * 3. Find-or-create nouns (value type heuristic for "has" objects)
 * 4. Find-or-create graph schema (name = noun concat, title = reading text)
 * 5. Find-or-create roles
 * 6. Delegate constraint lines to createWithHook('constraints', ...)
 */
export async function readingAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: HookContext,
): Promise<HookResult> {
  const rawText = doc.text || ''
  if (!rawText.trim()) return EMPTY_RESULT

  const lines = rawText.split('\n')
  const factLine = lines[0].trim().replace(/\.$/, '')
  const constraintLines = lines.slice(1)
    .filter((l: string) => l.match(/^\s+\S/))
    .map((l: string) => l.trim())

  const result: HookResult = { created: {}, warnings: [] }
  const domainId = context.domainId || doc.domain

  // Refresh nouns to pick up any created in this batch
  let nouns = context.allNouns.length > 0
    ? [...context.allNouns]
    : await refreshNouns(db, domainId)

  // Tokenize to find nouns in the reading
  const tokenized = tokenizeReading(factLine, nouns)
  let nounNames = tokenized.nounRefs.map(r => r.name)

  // If tokenization found fewer than 2 nouns, try extracting PascalCase words
  if (nounNames.length < 2) {
    const pascalWords = factLine.match(/[A-Z][a-zA-Z0-9]*/g) || []
    nounNames = pascalWords
  }

  if (nounNames.length < 2) {
    result.warnings.push(`Reading "${factLine}" has fewer than 2 nouns — skipping`)
    return result
  }

  // Determine predicate for entity/value heuristic
  const predicate = tokenized.predicate || ''
  const isHasPredicate = /^has$/i.test(predicate.trim())

  // Find-or-create nouns
  const nounIds: string[] = []
  for (let i = 0; i < nounNames.length; i++) {
    const name = nounNames[i]
    const existing = nouns.find(n => n.name === name)
    if (existing) {
      nounIds.push(existing.id)
    } else {
      // Heuristic: object of "has" → value type, otherwise entity
      const objectType = (isHasPredicate && i === nounNames.length - 1) ? 'value' : 'entity'
      const { doc: nounDoc } = await ensure(
        db, 'nouns',
        { name: { equals: name }, domain_id: { equals: domainId } },
        { name, objectType, domain: domainId },
      )
      nounIds.push(nounDoc.id)
      nouns.push({ name, id: nounDoc.id })
      result.created['nouns'] = [...(result.created['nouns'] || []), nounDoc]
    }
  }

  // Update context nouns for downstream hooks
  context.allNouns = nouns

  // Find-or-create graph schema
  const schemaName = nounNames.join('')
  const { doc: schema, created: schemaCreated } = await ensure(
    db, 'graph-schemas',
    { name: { equals: schemaName }, domain_id: { equals: domainId } },
    { name: schemaName, title: factLine, domain: domainId },
  )
  if (schemaCreated) {
    result.created['graph-schemas'] = [schema]
  }

  // Link reading to graph schema
  await db.updateInCollection('readings', doc.id, { graphSchema: schema.id })

  // Find-or-create roles
  for (let i = 0; i < nounIds.length; i++) {
    const { doc: role, created: roleCreated } = await ensure(
      db, 'roles',
      {
        reading_id: { equals: doc.id },
        noun_id: { equals: nounIds[i] },
        role_index: { equals: i },
      },
      {
        reading: doc.id,
        noun: nounIds[i],
        graphSchema: schema.id,
        roleIndex: i,
      },
    )
    if (roleCreated) {
      result.created['roles'] = [...(result.created['roles'] || []), role]
    }
  }

  // Delegate constraint lines
  for (const constraintText of constraintLines) {
    try {
      const { hookResult } = await createWithHook(
        db, 'constraints',
        { text: constraintText, domain: domainId },
        context,
      )
      // Merge sub-results
      for (const [key, docs] of Object.entries(hookResult.created)) {
        result.created[key] = [...(result.created[key] || []), ...docs]
      }
      result.warnings.push(...hookResult.warnings)
    } catch (err: any) {
      if (context.batch) {
        context.deferred = context.deferred || []
        context.deferred.push({
          data: { text: constraintText, domain: domainId },
          error: err.message,
        })
      } else {
        result.warnings.push(`Constraint rejected: ${constraintText} — ${err.message}`)
      }
    }
  }

  return result
}
