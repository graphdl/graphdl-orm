export interface QueryPathStep {
  from: string
  predicate: string
  to: string
  inverse: boolean
}

export interface ConceptualQueryResult {
  path: QueryPathStep[]
  filters: Array<{ field: string; value: string }>
  rootNoun?: string
}

export interface Reading {
  text: string
  nouns: string[] // nouns[0] is the "subject" noun, nouns[1+] are object nouns
}

/**
 * Extract quoted literal values (e.g. 'High') from the query text.
 * Returns the stripped query and the extracted values in order.
 */
function extractFilters(query: string): {
  stripped: string
  values: string[]
} {
  const values: string[] = []
  const stripped = query.replace(/'([^']+)'/g, (_match, val: string) => {
    values.push(val)
    return ''
  })
  return { stripped: stripped.replace(/\s+/g, ' ').trim(), values }
}

/**
 * Find all known nouns that appear in a text segment.
 * Matches longest noun names first to avoid partial matches
 * (e.g. "Support Request" before "Support").
 */
function findNounsInSegment(segment: string, nouns: string[]): string[] {
  // Sort by length descending for longest-first matching
  const sorted = [...nouns].sort((a, b) => b.length - a.length)
  const found: string[] = []
  let remaining = segment

  for (const noun of sorted) {
    // Case-insensitive search for the noun in the remaining text
    const idx = remaining.toLowerCase().indexOf(noun.toLowerCase())
    if (idx !== -1) {
      found.push(noun)
      // Remove the matched noun to prevent overlapping matches
      remaining =
        remaining.slice(0, idx) + remaining.slice(idx + noun.length)
    }
  }

  return found
}

/**
 * Find a reading that connects two nouns, checking both forward and inverse directions.
 *
 * Forward: `from` is nouns[0] in the reading (the subject position).
 * Inverse: `from` appears in the reading but is NOT nouns[0].
 */
function findReading(
  from: string,
  to: string,
  readings: Reading[],
): { reading: Reading; inverse: boolean } | undefined {
  // Forward: from is the first noun in the reading
  const forward = readings.find(
    (r) =>
      r.nouns.length >= 2 &&
      r.nouns[0].toLowerCase() === from.toLowerCase() &&
      r.nouns.some((n) => n.toLowerCase() === to.toLowerCase()),
  )
  if (forward) return { reading: forward, inverse: false }

  // Inverse: from appears in the reading but is not the first noun
  const inverse = readings.find(
    (r) =>
      r.nouns.length >= 2 &&
      r.nouns.some((n) => n.toLowerCase() === from.toLowerCase()) &&
      r.nouns.some((n) => n.toLowerCase() === to.toLowerCase()) &&
      r.nouns[0].toLowerCase() !== from.toLowerCase(),
  )
  if (inverse) return { reading: inverse, inverse: true }

  return undefined
}

/**
 * Extract the predicate text from a reading given the two nouns involved.
 * For "Customer submits Support Request", the predicate is "submits".
 */
function extractPredicate(reading: Reading): string {
  let text = reading.text
  // Remove nouns from the reading text to isolate the predicate(s).
  // Sort nouns by length descending to remove longest first.
  const sorted = [...reading.nouns].sort((a, b) => b.length - a.length)
  for (const noun of sorted) {
    text = text.replace(new RegExp(escapeRegex(noun), 'gi'), '')
  }
  return text.replace(/\s+/g, ' ').trim()
}

function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Resolve a conceptual ORM query into a structured path of reading hops with filters.
 *
 * Algorithm:
 * 1. Extract quoted literal values ('High') as filters
 * 2. Split query on " that " to get segments
 * 3. Find nouns in each segment (longest-first matching)
 * 4. For each consecutive pair of nouns, find a reading connecting them (forward or inverse)
 * 5. Build path of QueryPathSteps
 * 6. Associate filters with the target noun of the segment where the filter value appeared
 */
export function resolveConceptualQuery(
  query: string,
  nouns: string[],
  readings: Reading[],
): ConceptualQueryResult {
  const { stripped, values: filterValues } = extractFilters(query)

  // Split on " that " to get segments
  const segments = stripped.split(/\s+that\s+/i)

  // Collect all nouns found across segments, in order
  const nounSequence: string[] = []

  // For each segment, find the nouns it contains
  const segmentNouns: string[][] = segments.map((seg) =>
    findNounsInSegment(seg, nouns),
  )

  // Build the ordered noun sequence from segment nouns.
  // The first segment contributes all its nouns.
  // Subsequent segments contribute nouns that aren't already the last noun in sequence.
  for (let i = 0; i < segmentNouns.length; i++) {
    for (const noun of segmentNouns[i]) {
      if (
        nounSequence.length === 0 ||
        nounSequence[nounSequence.length - 1].toLowerCase() !==
          noun.toLowerCase()
      ) {
        nounSequence.push(noun)
      }
    }
  }

  // If we found fewer than 1 known noun, return empty result
  if (nounSequence.length === 0) {
    return { path: [], filters: [] }
  }

  if (nounSequence.length === 1) {
    return { path: [], filters: [], rootNoun: nounSequence[0] }
  }

  // Build path steps from the noun sequence.
  // For each target noun, try the previous noun first; if no reading exists,
  // walk backward through earlier nouns to find one that connects.
  // This handles fan-out queries like "X that has A that has B" where both
  // A and B connect to X rather than chaining A -> B.
  const path: QueryPathStep[] = []
  const rootNoun = nounSequence[0]

  for (let i = 1; i < nounSequence.length; i++) {
    const to = nounSequence[i]
    let matched = false

    // Try connecting from the immediately preceding noun first, then walk back
    for (let j = i - 1; j >= 0; j--) {
      const from = nounSequence[j]
      const match = findReading(from, to, readings)
      if (match) {
        const predicate = extractPredicate(match.reading)
        path.push({
          from,
          predicate,
          to,
          inverse: match.inverse,
        })
        matched = true
        break
      }
    }

    if (!matched) {
      // Can't find any reading connecting to this noun — skip it
      continue
    }
  }

  // If we couldn't build any path, and the nouns weren't recognized via readings,
  // return empty
  if (path.length === 0) {
    return { path: [], filters: [] }
  }

  // Map filter values to the target noun of the path step they belong to.
  // Strategy: walk through segments that have filter values (identified by
  // the original query having a quoted value in that segment), and associate
  // the filter with the last noun in that segment.
  const filters: Array<{ field: string; value: string }> = []

  // Re-split the original query on " that " to find which segments have filter values
  const originalSegments = query.split(/\s+that\s+/i)
  let filterIdx = 0
  for (const seg of originalSegments) {
    // Count quoted values in this segment
    const quotedMatches = seg.match(/'([^']+)'/g)
    if (quotedMatches) {
      // Find the last noun in this segment to use as the field
      const segNouns = findNounsInSegment(
        seg.replace(/'[^']+'/g, ''),
        nouns,
      )
      const fieldNoun = segNouns[segNouns.length - 1]

      for (const _qm of quotedMatches) {
        if (filterIdx < filterValues.length && fieldNoun) {
          filters.push({ field: fieldNoun, value: filterValues[filterIdx] })
        }
        filterIdx++
      }
    }
  }

  return { path, filters, rootNoun }
}
