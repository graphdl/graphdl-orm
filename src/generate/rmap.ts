/**
 * RMap pure functions — predicate parsing, property naming, noun tokenization.
 *
 * Ported from Generator.ts.bak (commit ddb8880) lines 2578-2841.
 * Zero external dependencies — only string manipulation.
 */

// ---------------------------------------------------------------------------
// NounRef — minimal interface for noun references used by RMap functions
// ---------------------------------------------------------------------------
export interface NounRef {
  id: string
  name?: string | null
  plural?: string | null
  objectType?: string
  valueType?: string | null
  format?: string | null
  pattern?: string | null
  enumValues?: string | null
  minimum?: number | null
  maximum?: number | null
  superType?: string | NounRef | null
  referenceScheme?: (string | NounRef)[] | null
}

// ---------------------------------------------------------------------------
// nameToKey — strips spaces/hyphens, replaces & with And
// ---------------------------------------------------------------------------
export function nameToKey(name: string): string {
  return name.replace(/[ \-]/g, '').replace(/&/g, 'And')
}

// ---------------------------------------------------------------------------
// transformPropertyName — PascalCase/ALL-CAPS → camelCase per RMap rules
// ---------------------------------------------------------------------------
export function transformPropertyName(propertyName?: string): string {
  if (!propertyName) return ''
  propertyName = nameToKey(propertyName)
  // Lowercase the whole string if it is all caps
  if (propertyName === propertyName.toUpperCase()) return propertyName.toLowerCase()
  // Handle leading uppercase runs (e.g., APIKey → apiKey, KBBId → kbbId, HTTPMethod → httpMethod)
  const leadingUpper = propertyName.match(/^[A-Z]+/)
  if (leadingUpper) {
    const run = leadingUpper[0]
    if (run.length === propertyName.length) return propertyName.toLowerCase()
    if (run.length > 1) return run.slice(0, -1).toLowerCase() + propertyName.slice(run.length - 1)
  }
  return propertyName[0].toLowerCase() + propertyName.slice(1).replace(/ /g, '')
}

// ---------------------------------------------------------------------------
// extractPropertyName — reading tokens → camelCase property name
// ---------------------------------------------------------------------------
export function extractPropertyName(objectReading: string[]): string {
  const propertyNamePrefix = objectReading[0].split(' ')
  const propertyName = transformPropertyName(
    propertyNamePrefix
      .map((n) => (n === n.toUpperCase() ? n[0].toUpperCase() + n.slice(1).toLowerCase() : n))
      .join('') +
      objectReading
        .slice(1)
        .map((r) => r[0].toUpperCase() + r.slice(1))
        .join(''),
  )
  return propertyName
}

// ---------------------------------------------------------------------------
// nounListToRegex — create regex matching any noun name (longest first)
// ---------------------------------------------------------------------------
export function nounListToRegex(nouns?: NounRef[]): RegExp {
  return nouns
    ? new RegExp(
        '(' +
          nouns
            .filter((n) => n.name)
            .map((n) => '\\b' + n.name + '\\b-?')
            .sort((a, b) => b.length - a.length)
            .join('|') +
          ')',
      )
    : new RegExp('')
}

// ---------------------------------------------------------------------------
// toPredicate — tokenize a reading string by noun names then by spaces
// ---------------------------------------------------------------------------
export function toPredicate({
  reading,
  nouns,
  nounRegex,
}: {
  reading: string
  nouns: NounRef[]
  nounRegex?: RegExp
}): string[] {
  return reading
    .split(nounRegex || nounListToRegex(nouns))
    .flatMap((token) =>
      nouns.find((n) => n.name === token.replace(/-$/, ''))
        ? token
        : token
            .trim()
            .split(' ')
            .map((word) => word.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase())),
    )
    .filter((word) => word)
}

// ---------------------------------------------------------------------------
// findPredicateObject — locate the object noun in a tokenized predicate
// ---------------------------------------------------------------------------
export function findPredicateObject({
  predicate,
  subject,
  object,
  plural,
}: {
  predicate: string[]
  subject: NounRef
  object?: NounRef
  plural?: string | null | undefined
}): { objectBegin: number; objectEnd: number } {
  let subjectIndex = predicate.indexOf(subject.name || '')
  if (subjectIndex === -1 && subject.name)
    subjectIndex = predicate.indexOf(subject.name + '-' || '')
  if (subjectIndex === -1) return { objectBegin: 0, objectEnd: 0 }

  let objectIndex = !object ? -1 : predicate.indexOf(object.name || '')
  if (object && objectIndex === -1 && object.name)
    objectIndex = predicate.indexOf(object.name + '-' || '')
  if (object && objectIndex === -1)
    throw new Error(`Object "${object.name}" not found in predicate "${predicate.join(' ')}"`)

  if (plural) predicate[objectIndex] = plural[0].toUpperCase() + plural.slice(1)
  let objectBegin: number, objectEnd: number
  if (objectIndex === -1) {
    objectBegin = subjectIndex + 1
    objectEnd = predicate.length
  } else if (subjectIndex < objectIndex) {
    objectBegin = subjectIndex + 1
    objectEnd = predicate[objectIndex].endsWith('-') ? predicate.length : objectIndex + 1
  } else {
    objectBegin = 0
    objectEnd = objectIndex + 1
  }
  while (objectIndex > -1 && !predicate[objectBegin].endsWith('-') && objectBegin < objectIndex - 1)
    objectBegin++
  // Skip the last pre-object token only if it's a verb/preposition (not a qualifier)
  if (objectBegin < objectIndex) {
    const token = predicate[objectBegin].toLowerCase()
    const verbsAndPrepositions = [
      'has', 'is', 'was', 'are', 'were', 'been',
      'to', 'via', 'from', 'for', 'on', 'of', 'in', 'at', 'by', 'with', 'as',
      'the', 'a', 'an',
      'belongs', 'arrives', 'leads', 'sources', 'includes', 'concerns',
      'submits', 'sends', 'affects', 'involves', 'authenticates',
      'manufactured', 'connects', 'charges', 'covers', 'data',
    ]
    if (verbsAndPrepositions.includes(token)) objectBegin++
  }
  return { objectBegin, objectEnd }
}
