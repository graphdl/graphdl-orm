/**
 * useGuards — fetch + CRUD for Guard entities that prevent specific
 * transitions in a State Machine Definition.
 *
 * readings/state.md defines:
 *   Guard(.Name) is an entity type.
 *   Guard references Fact Type.
 *   Guard prevents Transition.
 *
 * We filter by `transition` (the transition id) so the editor can
 * list only the guards attached to a given transition. Create /
 * delete flow through the standard /arest/guards[/<id>] data
 * provider calls.
 */
import { useMemo } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createArestDataProvider } from '../providers'

export interface ArestGuard {
  /** Guard.Name is the primary key per readings/state.md. */
  id: string
  /** Transition id this guard prevents. */
  transition?: string
  /** Fact Type id the guard references. */
  factType?: string
  /** Optional description / expression the guard evaluates. */
  expression?: string
}

export interface UseGuardsOptions {
  baseUrl: string
  fetch?: typeof globalThis.fetch
  /** Field on the Guard entity that stores the transition reference. */
  transitionField?: string
}

export interface UseGuardsResult {
  guards: ArestGuard[]
  isLoading: boolean
  error?: unknown
  addGuard: (g: ArestGuard) => Promise<void>
  deleteGuard: (id: string) => Promise<void>
}

const RESOURCE = 'guards'

export function useGuards(transitionId: string, options: UseGuardsOptions): UseGuardsResult {
  const transitionField = options.transitionField ?? 'transition'
  const provider = useMemo(
    () => createArestDataProvider({ baseUrl: options.baseUrl, fetch: options.fetch }),
    [options.baseUrl, options.fetch],
  )
  const queryClient = useQueryClient()

  const query = useQuery({
    queryKey: ['arest', 'list', RESOURCE, { [transitionField]: transitionId }],
    queryFn: () => provider.getList<ArestGuard>(RESOURCE, {
      filter: { [transitionField]: transitionId },
    }),
    enabled: transitionId !== '',
  })

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['arest', 'list', RESOURCE] })

  const addMutation = useMutation({
    mutationFn: (g: ArestGuard) =>
      provider.create<ArestGuard>(RESOURCE, {
        data: { ...g, [transitionField]: transitionId } as Partial<ArestGuard>,
      }),
    onSuccess: invalidate,
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => provider.delete(RESOURCE, { id }),
    onSuccess: invalidate,
  })

  return {
    guards: query.data?.data ?? [],
    isLoading: query.isLoading,
    error: query.error ?? undefined,
    addGuard: async (g) => { await addMutation.mutateAsync(g) },
    deleteGuard: async (id) => { await deleteMutation.mutateAsync(id) },
  }
}
