/**
 * usePermissions — thin wrapper over arestAuthProvider.getPermissions
 * that exposes role gating as a React hook.
 *
 * Permission strings follow the format the auth provider emits:
 *   `<FactType>:<EntityId>`
 * for example `User_owns_Organization:acme`. The `has` helper
 * supports both exact matching and pattern matching (wildcard after
 * ':'). Callers get a loading flag so UI can distinguish "waiting
 * on auth" from "no permission".
 */
import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { createArestAuthProvider } from '../providers'

export interface UsePermissionsOptions {
  baseUrl: string
  fetch?: typeof globalThis.fetch
}

export interface UsePermissionsResult {
  permissions: string[]
  isLoading: boolean
  has: (pattern: string) => boolean
}

export function usePermissions(options: UsePermissionsOptions): UsePermissionsResult {
  const provider = useMemo(
    () => createArestAuthProvider({ baseUrl: options.baseUrl, fetch: options.fetch }),
    [options.baseUrl, options.fetch],
  )

  const query = useQuery({
    queryKey: ['arest', 'permissions'],
    queryFn: () => provider.getPermissions(),
    // Permissions change rarely; keep warm for a minute.
    staleTime: 60_000,
  })

  const permissions = query.data ?? []

  const has = (pattern: string): boolean => {
    if (pattern === '*') return permissions.length > 0
    // Exact match.
    if (permissions.includes(pattern)) return true
    // Pattern match: "User_owns_Organization:*" matches any
    // permission whose fact-type prefix equals the left side.
    const starIdx = pattern.indexOf('*')
    if (starIdx >= 0) {
      const prefix = pattern.slice(0, starIdx)
      return permissions.some((p) => p.startsWith(prefix))
    }
    return false
  }

  return { permissions, isLoading: query.isLoading, has }
}
