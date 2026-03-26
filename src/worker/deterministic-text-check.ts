/**
 * Deterministic text constraint checker.
 *
 * Checks response text against deontic constraints that reference value types
 * with concrete enum values. Pure string matching — no LLM, no FOL engine.
 *
 * Three-tier evaluation:
 * 1. Deterministic text check (this module) — string presence/absence
 * 2. FOL engine — structural population constraints
 * 3. LLM semantic checker — judgment-requiring constraints
 */

export interface TextConstraint {
  constraintId: string
  text: string
  operator: 'forbidden' | 'obligatory'
  values: string[]  // enum values to check for
}

export interface TextViolation {
  constraintId: string
  constraintText: string
  operator: string
  value: string       // the specific enum value that matched/was missing
  evidence: string    // the surrounding text where the match was found
}

/**
 * Check response text against deterministic text constraints.
 *
 * - forbidden + value found in text = violation
 * - obligatory + value NOT found in text = violation
 */
export function checkDeterministicText(
  responseText: string,
  constraints: TextConstraint[],
): TextViolation[] {
  const violations: TextViolation[] = []

  for (const constraint of constraints) {
    for (const value of constraint.values) {
      const found = responseText.includes(value)

      if (constraint.operator === 'forbidden' && found) {
        // Find the evidence — surrounding context
        const idx = responseText.indexOf(value)
        const start = Math.max(0, idx - 40)
        const end = Math.min(responseText.length, idx + value.length + 40)
        const evidence = responseText.slice(start, end).trim()

        violations.push({
          constraintId: constraint.constraintId,
          constraintText: constraint.text,
          operator: 'forbidden',
          value,
          evidence,
        })
      }

      if (constraint.operator === 'obligatory' && !found) {
        violations.push({
          constraintId: constraint.constraintId,
          constraintText: constraint.text,
          operator: 'obligatory',
          value,
          evidence: `Value '${value}' not found in response text`,
        })
      }
    }
  }

  return violations
}

/**
 * Build TextConstraints from constraint entities and their referenced nouns.
 *
 * A constraint is deterministically checkable if:
 * - It's deontic (forbidden/obligatory)
 * - Its text references a noun with concrete enum values
 * - Those values are literal strings that can be searched for
 */
export function buildTextConstraints(
  constraints: Array<{ id: string; data: Record<string, unknown> }>,
  nouns: Array<{ id: string; data: Record<string, unknown> }>,
): TextConstraint[] {
  const nounsByName = new Map<string, { id: string; data: Record<string, unknown> }>()
  for (const noun of nouns) {
    const name = (noun.data.name as string) || ''
    if (name) nounsByName.set(name, noun)
  }

  const result: TextConstraint[] = []

  for (const constraint of constraints) {
    const d = constraint.data
    if (d.modality !== 'Deontic') continue
    const text = (d.text as string) || ''
    if (!text) continue

    // Determine operator from text
    let operator: 'forbidden' | 'obligatory' | null = null
    if (text.toLowerCase().includes('forbidden')) operator = 'forbidden'
    else if (text.toLowerCase().includes('obligatory')) operator = 'obligatory'
    if (!operator) continue

    // Find nouns referenced in the constraint text that have enum values
    for (const [nounName, noun] of nounsByName) {
      if (!text.includes(nounName)) continue

      const enumValues = noun.data.enumValues || noun.data.enum_values || noun.data.enum
      if (!enumValues) continue

      let values: string[]
      if (typeof enumValues === 'string') {
        // Try JSON array first, then comma-separated
        try {
          const parsed = JSON.parse(enumValues)
          values = Array.isArray(parsed) ? parsed.map(String) : [String(parsed)]
        } catch {
          // Comma-separated: "**, ##, - , ```"
          values = enumValues.split(',').map(v => v.trim()).filter(Boolean)
        }
      } else if (Array.isArray(enumValues)) {
        values = enumValues.map(String)
      } else {
        continue
      }

      if (values.length === 0) continue

      result.push({
        constraintId: constraint.id,
        text,
        operator,
        values,
      })
    }
  }

  return result
}
