/**
 * useStateMachine — fetch an AREST State Machine Definition together
 * with its Transitions in a single hook, and expose mutation helpers
 * for CRUD on the transition facts.
 *
 * URLs (per AREST's nounToSlug convention):
 *   GET    /arest/state-machine-definitions/<smdId>
 *   GET    /arest/transitions?filter[stateMachineDefinition]=<smdId>
 *   POST   /arest/transitions     — add a new transition
 *   PATCH  /arest/transitions/<id>
 *   DELETE /arest/transitions/<id>
 *
 * Invalidation: mutations invalidate both the SMD one-query and the
 * transitions list-query, and the SSE bridge invalidates the same
 * keys on broadcast. So any other tab editing the machine converges
 * within the broadcast budget.
 */
import { useMemo } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createArestDataProvider } from '../providers'
import type { ArestStateMachineDefinition, ArestTransition } from './xstateConfig'

export interface UseStateMachineOptions {
  baseUrl: string
  fetch?: typeof globalThis.fetch
}

export interface UseStateMachineResult {
  smd?: ArestStateMachineDefinition
  transitions: ArestTransition[]
  isLoading: boolean
  error?: unknown
  addTransition: (t: Omit<ArestTransition, 'id'> & { id: string }) => Promise<void>
  updateTransition: (id: string, patch: Partial<ArestTransition>) => Promise<void>
  deleteTransition: (id: string) => Promise<void>
}

const SMD_RESOURCE = 'state-machine-definitions'
const TRANSITION_RESOURCE = 'transitions'

export function useStateMachine(
  smdId: string,
  options: UseStateMachineOptions,
): UseStateMachineResult {
  const provider = useMemo(
    () => createArestDataProvider({ baseUrl: options.baseUrl, fetch: options.fetch }),
    [options.baseUrl, options.fetch],
  )
  const queryClient = useQueryClient()

  const smdQuery = useQuery<{ data: ArestStateMachineDefinition }>({
    queryKey: ['arest', 'one', SMD_RESOURCE, smdId],
    queryFn: () => provider.getOne<ArestStateMachineDefinition>(SMD_RESOURCE, { id: smdId }),
    enabled: smdId !== '',
  })

  const transitionsQuery = useQuery({
    queryKey: ['arest', 'list', TRANSITION_RESOURCE, { stateMachineDefinition: smdId }],
    queryFn: () => provider.getList<ArestTransition>(TRANSITION_RESOURCE, {
      filter: { stateMachineDefinition: smdId },
    }),
    enabled: smdId !== '',
  })

  const invalidate = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ['arest', 'list', TRANSITION_RESOURCE] }),
      queryClient.invalidateQueries({ queryKey: ['arest', 'one', SMD_RESOURCE, smdId] }),
    ])
  }

  const addMutation = useMutation({
    mutationFn: (t: ArestTransition) =>
      provider.create<ArestTransition>(TRANSITION_RESOURCE, {
        data: { ...t, stateMachineDefinition: smdId } as Partial<ArestTransition>,
      }),
    onSuccess: invalidate,
  })

  const updateMutation = useMutation({
    mutationFn: (args: { id: string; patch: Partial<ArestTransition> }) =>
      provider.update<ArestTransition>(TRANSITION_RESOURCE, {
        id: args.id,
        data: args.patch,
      }),
    onSuccess: invalidate,
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => provider.delete(TRANSITION_RESOURCE, { id }),
    onSuccess: invalidate,
  })

  return {
    smd: smdQuery.data?.data,
    transitions: transitionsQuery.data?.data ?? [],
    isLoading: smdQuery.isLoading || transitionsQuery.isLoading,
    error: smdQuery.error ?? transitionsQuery.error ?? undefined,
    addTransition: async (t) => {
      await addMutation.mutateAsync(t as ArestTransition)
    },
    updateTransition: async (id, patch) => {
      await updateMutation.mutateAsync({ id, patch })
    },
    deleteTransition: async (id) => {
      await deleteMutation.mutateAsync(id)
    },
  }
}
