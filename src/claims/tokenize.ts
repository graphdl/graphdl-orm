/**
 * Single Noun Tokenizer
 *
 * Replaces three duplicate tokenization implementations:
 * - src/parse/forml2.ts:buildNounRegex()
 * - src/collections/Readings.ts afterChange hook (lines 52-59)
 * - src/seed/handler.ts:applySubsetConstraint() (line 393)
 *
 * Pure function — no Payload dependency.
 */

export interface NounRef {
  name: string
  id: string
  index: number
}

export interface TokenizeResult {
  nounRefs: NounRef[]
  predicate: string
  /** Property name derived from hyphen binding, e.g., "created- at Date" → "createdAtDate" */
  boundPropertyName?: string
}

/**
 * Tokenize a reading text to find known nouns in order of appearance.
 *
 * Uses longest-first matching to avoid partial matches
 * (e.g., "SupportRequest" matches before "Request").
 *
 * @param text    - The reading text, e.g. "Customer submits SupportRequest"
 * @param nouns   - Known nouns with their IDs
 * @returns       - Found nouns (in order, with positional index) and predicate
 */
export function tokenizeReading(
  text: string,
  nouns: Array<{ name: string; id: string }>,
): TokenizeResult {
  if (nouns.length === 0) {
    return { nounRefs: [], predicate: '' }
  }

  // Sort longest first so "SupportRequest" matches before "Request"
  const sorted = [...nouns].sort((a, b) => b.name.length - a.name.length)

  // Build regex with word boundaries, escaping special regex characters
  const pattern = sorted.map((n) => n.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')
  const regex = new RegExp('\\b(' + pattern + ')\\b', 'g')

  // Find all noun matches in order of appearance
  const nounRefs: NounRef[] = []
  let match: RegExpExecArray | null
  let index = 0
  while ((match = regex.exec(text)) !== null) {
    const matchedNoun = nouns.find((n) => n.name === match![1])
    if (matchedNoun) {
      nounRefs.push({
        name: matchedNoun.name,
        id: matchedNoun.id,
        index: index++,
      })
    }
  }

  // Extract predicate: text between first and second noun
  let predicate = ''
  if (nounRefs.length >= 2) {
    const firstNounEnd = text.indexOf(nounRefs[0].name) + nounRefs[0].name.length
    const secondNounStart = text.indexOf(nounRefs[1].name, firstNounEnd)
    if (secondNounStart > firstNounEnd) {
      predicate = text.slice(firstNounEnd, secondNounStart).trim()
    }
  }

  // Hyphen binding: "was created- at Date" → boundPropertyName = "createdAtDate"
  // A hyphen at the end of a word binds the following word(s) to the object noun
  let boundPropertyName: string | undefined
  const hyphenMatch = predicate.match(/(\w+)-\s+(.+)$/)
  if (hyphenMatch && nounRefs.length >= 2) {
    const boundWord = hyphenMatch[1]         // "created"
    const trailingWords = hyphenMatch[2]     // "at" (words between hyphen and object noun)
    const objectNoun = nounRefs[nounRefs.length - 1].name  // "Date"
    // Build camelCase property: "created" + "At" + "Date"
    const parts = trailingWords.split(/\s+/).filter(Boolean)
    boundPropertyName = boundWord + parts.map(w => w.charAt(0).toUpperCase() + w.slice(1)).join('') +
      objectNoun.replace(/\s+/g, '')
  }

  return { nounRefs, predicate, boundPropertyName }
}
