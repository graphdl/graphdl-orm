import { parseReading, type ParsedReading } from './forml2'

export interface ParseResult {
  readings: ParsedReading[]
  newNounCandidates: string[]
}

export function parseText(text: string, knownNouns: string[]): ParseResult {
  const lines = text
    .split('\n')
    .map(l => l.trim())
    .filter(l => l.length > 0 && !l.startsWith('#'))

  const readings: ParsedReading[] = []
  const allNouns = new Set(knownNouns)
  const newNounCandidates = new Set<string>()

  for (const line of lines) {
    const parsed = parseReading(line, [...allNouns])
    readings.push(parsed)

    // Detect capitalized words that aren't known nouns as candidates
    const capitalizedWords = line.match(/\b([A-Z][a-zA-Z]+)\b/g) || []
    for (const word of capitalizedWords) {
      if (!allNouns.has(word) && !isConstraintKeyword(word)) {
        newNounCandidates.add(word)
        allNouns.add(word) // Use for subsequent lines
      }
    }
  }

  return { readings, newNounCandidates: [...newNounCandidates] }
}

const CONSTRAINT_KEYWORDS = new Set([
  'Each', 'It', 'That', 'The',
])

function isConstraintKeyword(word: string): boolean {
  return CONSTRAINT_KEYWORDS.has(word)
}
