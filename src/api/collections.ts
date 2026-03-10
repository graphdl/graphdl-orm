/**
 * Payload-compatible REST API handlers for collections.
 *
 * Translates URL query params (where[], sort, limit, page, depth)
 * into GraphDLDB collection queries.
 */

/**
 * Parse Payload-style where[] query params into a nested where object.
 *
 * Examples:
 *   where[name][equals]=Test        → { name: { equals: 'Test' } }
 *   where[or][0][name][equals]=A    → { or: [{ name: { equals: 'A' } }] }
 *   where[and][0][x][equals]=1      → { and: [{ x: { equals: '1' } }] }
 */
export function parsePayloadWhereParams(params: URLSearchParams): Record<string, any> {
  const where: Record<string, any> = {}

  for (const [key, value] of params.entries()) {
    if (!key.startsWith('where[')) continue

    // Parse bracket path: where[a][b][c] → ['a', 'b', 'c']
    const path = key.slice(6).replace(/\]$/, '').split('][')
    if (path.length === 0) continue

    setNestedValue(where, path, value)
  }

  return where
}

/** Set a value at a nested path, creating arrays for numeric keys. */
function setNestedValue(obj: any, path: string[], value: any): void {
  let current = obj

  for (let i = 0; i < path.length - 1; i++) {
    const key = path[i]
    const nextKey = path[i + 1]
    const isNextNumeric = /^\d+$/.test(nextKey)

    if (!(key in current)) {
      current[key] = isNextNumeric ? [] : {}
    }
    current = current[key]
  }

  const lastKey = path[path.length - 1]
  current[lastKey] = value
}

/**
 * Parse standard Payload query options from URL params.
 */
export function parseQueryOptions(params: URLSearchParams): {
  where: Record<string, any>
  limit: number
  page: number
  sort: string | undefined
  depth: number
} {
  const where = parsePayloadWhereParams(params)
  const limit = Math.min(parseInt(params.get('limit') || '100', 10), 1000)
  const page = Math.max(parseInt(params.get('page') || '1', 10), 1)
  const sort = params.get('sort') || undefined
  const depth = parseInt(params.get('depth') || '0', 10)

  return { where, limit, page, sort, depth }
}
