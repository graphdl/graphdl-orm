/**
 * Resource hooks — useArestList / useArestOne / useArestCreate /
 * useArestUpdate / useArestDelete.
 *
 * Each wraps the corresponding arestDataProvider method in a TanStack
 * Query hook whose queryKey matches the family the SSE bridge
 * invalidates on broadcast (see arestQueryBridge.ts). Callers supply
 * a noun ("Organization") rather than the slug ("organizations"); the
 * hook slugifies once via the shared nounToSlug convention.
 */
import { useMemo } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from '@tanstack/react-query'
import { nounToSlug } from '../query'
import {
  createArestDataProvider,
  type CreateParams,
  type CreateResult,
  type GetListParams,
  type GetListResult,
  type GetOneResult,
  type UpdateParams,
  type UpdateResult,
} from '../providers'

export interface ArestResourceOptions {
  baseUrl: string
  fetch?: typeof globalThis.fetch
}

function useProvider(opts: ArestResourceOptions) {
  return useMemo(
    () => createArestDataProvider({ baseUrl: opts.baseUrl, fetch: opts.fetch }),
    [opts.baseUrl, opts.fetch],
  )
}

export function useArestList<T = unknown>(
  noun: string,
  params: GetListParams | undefined,
  opts: ArestResourceOptions,
): UseQueryResult<GetListResult<T>> {
  const resource = nounToSlug(noun)
  const provider = useProvider(opts)
  return useQuery<GetListResult<T>>({
    queryKey: params === undefined
      ? ['arest', 'list', resource]
      : ['arest', 'list', resource, params],
    queryFn: () => provider.getList<T>(resource, params),
  })
}

export function useArestOne<T = unknown>(
  noun: string,
  id: string,
  opts: ArestResourceOptions,
): UseQueryResult<GetOneResult<T>> {
  const resource = nounToSlug(noun)
  const provider = useProvider(opts)
  return useQuery<GetOneResult<T>>({
    queryKey: ['arest', 'one', resource, id],
    queryFn: () => provider.getOne<T>(resource, { id }),
    enabled: id !== '',
  })
}

export interface UseArestCreateResult<T> {
  create: (data: Partial<T>) => Promise<CreateResult<T>>
  isPending: boolean
  error: unknown
}

export function useArestCreate<T = unknown>(
  noun: string,
  opts: ArestResourceOptions,
): UseArestCreateResult<T> {
  const resource = nounToSlug(noun)
  const provider = useProvider(opts)
  const queryClient = useQueryClient()

  const mutation = useMutation<CreateResult<T>, Error, CreateParams<T>>({
    mutationFn: (params) => provider.create<T>(resource, params),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['arest', 'list', resource] })
    },
  })

  return {
    create: (data: Partial<T>) => mutation.mutateAsync({ data }),
    isPending: mutation.isPending,
    error: mutation.error,
  }
}

export interface UseArestUpdateResult<T> {
  update: (data: Partial<T>) => Promise<UpdateResult<T>>
  isPending: boolean
  error: unknown
}

export function useArestUpdate<T = unknown>(
  noun: string,
  id: string,
  opts: ArestResourceOptions,
): UseArestUpdateResult<T> {
  const resource = nounToSlug(noun)
  const provider = useProvider(opts)
  const queryClient = useQueryClient()

  const mutation = useMutation<UpdateResult<T>, Error, UpdateParams<T>>({
    mutationFn: (params) => provider.update<T>(resource, params),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['arest', 'list', resource] }),
        queryClient.invalidateQueries({ queryKey: ['arest', 'one', resource, id] }),
      ])
    },
  })

  return {
    update: (data: Partial<T>) => mutation.mutateAsync({ id, data }),
    isPending: mutation.isPending,
    error: mutation.error,
  }
}
