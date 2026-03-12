import { ensure, type HookResult, EMPTY_RESULT } from './index'

const SUBTYPE_PATTERN = /^(\S+)\s+is a subtype of\s+(\S+)/i

export async function nounAfterCreate(
  db: any,
  doc: Record<string, any>,
  context: { domainId: string; allNouns: Array<{ name: string; id: string }> },
): Promise<HookResult> {
  const text = doc.promptText || ''
  const match = text.match(SUBTYPE_PATTERN)
  if (!match) return EMPTY_RESULT

  const parentName = match[2].replace(/\.$/, '')
  const result: HookResult = { created: {}, warnings: [] }

  // Find or create the parent noun
  let parentId: string | undefined
  const existing = context.allNouns.find(n => n.name === parentName)
  if (existing) {
    parentId = existing.id
  } else {
    const { doc: parentDoc, created } = await ensure(
      db, 'nouns',
      { name: { equals: parentName }, domain_id: { equals: context.domainId } },
      { name: parentName, objectType: 'entity', domain: context.domainId },
    )
    parentId = parentDoc.id
    if (created) {
      result.created['nouns'] = [parentDoc]
    }
  }

  // Set the superType FK
  if (parentId) {
    await db.updateInCollection('nouns', doc.id, { superType: parentId })
  }

  return result
}
