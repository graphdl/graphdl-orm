/**
 * createArestQueryClient — a QueryClient preset wired for AREST's
 * cache-invalidation-over-SSE model.
 *
 * Defaults:
 *   staleTime: 0   so invalidateQueries always refetches immediately
 *                  when a matching CellEvent arrives. Without the SSE
 *                  bridge you probably want a non-zero staleTime; with
 *                  it, the server authoritatively says "this key
 *                  changed" and we honor that right away.
 *   retry:    1    one retry on transient errors; the worker is
 *                  colocated so a second failure is usually real.
 *   refetchOnWindowFocus: false  the SSE bridge makes this redundant
 *                                and the extra re-fetches confuse the
 *                                event-sourcing story.
 */
import { QueryClient } from '@tanstack/react-query'

export function createArestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 0,
        retry: 1,
        refetchOnWindowFocus: false,
      },
      mutations: {
        retry: 0,
      },
    },
  })
}
