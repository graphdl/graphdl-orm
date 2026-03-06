import type { DeonticConstraintGroup } from '../seed/deontic'

export interface ConstraintMatcher {
  constraintText: string
  instances: string[]
  regex: RegExp | null
}

export interface MatchResult {
  factType: string
  instance: string
  span: [number, number]
}

export interface MatchOutput {
  matches: MatchResult[]
  unmatchedConstraints: string[]
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function isAlphanumericStart(s: string): boolean {
  return /^[a-zA-Z0-9]/.test(s)
}

export function buildMatchers(groups: DeonticConstraintGroup[]): ConstraintMatcher[] {
  return groups.map((group) => {
    if (group.instances.length === 0) {
      return {
        constraintText: group.constraintText,
        instances: [],
        regex: null,
      }
    }

    const sorted = [...group.instances].sort((a, b) => b.length - a.length)

    const alternatives = sorted.map((inst) => {
      const escaped = escapeRegex(inst)
      if (isAlphanumericStart(inst)) {
        return `\\b${escaped}\\b`
      }
      return escaped
    })

    return {
      constraintText: group.constraintText,
      instances: group.instances,
      regex: new RegExp(alternatives.join('|'), 'gi'),
    }
  })
}

export function matchText(text: string, matchers: ConstraintMatcher[]): MatchOutput {
  const matches: MatchResult[] = []
  const unmatchedConstraints: string[] = []

  for (const matcher of matchers) {
    if (matcher.regex === null) {
      unmatchedConstraints.push(matcher.constraintText)
      continue
    }

    // Reset lastIndex for global regex
    matcher.regex.lastIndex = 0
    let match: RegExpExecArray | null

    while ((match = matcher.regex.exec(text)) !== null) {
      const matchedText = match[0]
      // Find the original instance via case-insensitive comparison
      const originalInstance = matcher.instances.find(
        (inst) => inst.toLowerCase() === matchedText.toLowerCase(),
      ) || matchedText

      matches.push({
        factType: matcher.constraintText,
        instance: originalInstance,
        span: [match.index, match.index + matchedText.length],
      })
    }
  }

  return { matches, unmatchedConstraints }
}
