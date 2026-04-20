/**
 * arestDataProvider — adapts @mdxui/admin's expected DataProvider
 * shape onto the AREST worker's /arest/{resource}[...] REST surface.
 *
 * Resource names are **plural slugs** (the same convention AREST's
 * `nounToSlug` uses). The provider speaks directly to the worker with
 * session-cookie auth (credentials: include). Responses come back in
 * the Theorem-5 envelope — see src/api/envelope.ts in the worker —
 * which we unwrap to `{ data }` for the caller, surfacing violations
 * as thrown errors.
 */
import type {
  ArestDataProvider,
  ArestEnvelope,
  CreateParams,
  CreateResult,
  DeleteManyParams,
  DeleteManyResult,
  DeleteParams,
  DeleteResult,
  GetListParams,
  GetListResult,
  GetManyParams,
  GetManyReferenceParams,
  GetManyReferenceResult,
  GetManyResult,
  GetOneParams,
  GetOneResult,
  Identifier,
  UpdateManyParams,
  UpdateManyResult,
  UpdateParams,
  UpdateResult,
} from './types'

export interface ArestDataProviderOptions {
  /** e.g. 'https://ui.auto.dev/arest' */
  baseUrl: string
  /**
   * Optional fetch override — tests inject one. Defaults to globalThis.fetch
   * so production code always uses the platform fetch.
   */
  fetch?: typeof globalThis.fetch
}

/**
 * Shape of a collection response straight off AREST's /arest/{slug}
 * collection handler:
 *   { type, docs, totalDocs, _links, _schema }
 * We normalize both the `{ docs }` shape and the Theorem-5 `{ data }`
 * envelope into the DataProvider-facing `{ data: T[], total }`.
 */
interface ListBody {
  data?: unknown
  docs?: unknown
  totalDocs?: number
  total?: number
  violations?: ArestEnvelope<unknown>['violations']
  _links?: ArestEnvelope<unknown>['_links']
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

function violationMessage(body: unknown): string | null {
  if (!isRecord(body)) return null
  const v = body.violations
  if (Array.isArray(v) && v.length > 0) {
    const first = v[0]
    if (isRecord(first)) {
      return (first.detail as string) || (first.reading as string) || null
    }
  }
  if (Array.isArray((body as { errors?: unknown }).errors)) {
    const errs = (body as { errors: unknown[] }).errors
    if (errs.length > 0 && isRecord(errs[0]) && typeof errs[0].message === 'string') {
      return errs[0].message as string
    }
  }
  return null
}

async function request(
  url: string,
  init: RequestInit,
  fetchImpl: typeof globalThis.fetch,
): Promise<unknown> {
  const response = await fetchImpl(url, {
    credentials: 'include',
    ...init,
    headers: {
      accept: 'application/json',
      ...(init.body ? { 'content-type': 'application/json' } : {}),
      ...(init.headers ?? {}),
    },
  })
  let body: unknown = null
  const text = await response.text()
  if (text) {
    try { body = JSON.parse(text) } catch { body = text }
  }
  if (!response.ok) {
    const msg = violationMessage(body) || `HTTP ${response.status}`
    throw new Error(msg)
  }
  return body
}

function unwrap<T>(body: unknown): T {
  if (isRecord(body) && 'data' in body) {
    return (body as { data: T }).data
  }
  return body as T
}

function normalizeList<T>(body: unknown): GetListResult<T> {
  if (!isRecord(body)) return { data: [] as T[] }

  const bodyRec = body as ListBody
  if (Array.isArray(bodyRec.docs)) {
    return {
      data: bodyRec.docs as T[],
      total: bodyRec.totalDocs ?? bodyRec.docs.length,
    }
  }

  const data = bodyRec.data
  if (Array.isArray(data)) {
    return {
      data: data as T[],
      total: bodyRec.total ?? data.length,
    }
  }

  if (isRecord(data) && Array.isArray((data as ListBody).docs)) {
    const inner = data as ListBody
    return {
      data: (inner.docs as T[]),
      total: inner.totalDocs ?? (inner.docs as T[]).length,
    }
  }

  return { data: [] as T[] }
}

function encodeId(id: Identifier): string {
  return encodeURIComponent(String(id))
}

function listQuery(params?: GetListParams): string {
  if (!params) return ''
  const qs = new URLSearchParams()
  if (params.pagination) {
    qs.set('page', String(params.pagination.page))
    qs.set('perPage', String(params.pagination.perPage))
  }
  if (params.sort) {
    qs.set('sort', params.sort.field)
    qs.set('order', params.sort.order)
  }
  if (params.filter) {
    for (const [key, value] of Object.entries(params.filter)) {
      if (value == null) continue
      qs.set(`filter[${key}]`, String(value))
    }
  }
  const s = qs.toString()
  return s ? `?${s}` : ''
}

export function createArestDataProvider(
  options: ArestDataProviderOptions,
): ArestDataProvider {
  const baseUrl = options.baseUrl.replace(/\/$/, '')
  const fetchImpl = options.fetch ?? ((...args) => globalThis.fetch(...args))

  const getList = async <T = unknown>(
    resource: string,
    params?: GetListParams,
  ): Promise<GetListResult<T>> => {
    const url = `${baseUrl}/${resource}${listQuery(params)}`
    const body = await request(url, { method: 'GET' }, fetchImpl)
    return normalizeList<T>(body)
  }

  const getOne = async <T = unknown>(
    resource: string,
    params: GetOneParams,
  ): Promise<GetOneResult<T>> => {
    const url = `${baseUrl}/${resource}/${encodeId(params.id)}`
    const body = await request(url, { method: 'GET' }, fetchImpl)
    return { data: unwrap<T>(body) }
  }

  const getMany = async <T = unknown>(
    resource: string,
    params: GetManyParams,
  ): Promise<GetManyResult<T>> => {
    const rows = await Promise.all(
      params.ids.map((id) => getOne<T>(resource, { id })),
    )
    return { data: rows.map((r) => r.data) }
  }

  const getManyReference = async <T = unknown>(
    resource: string,
    params: GetManyReferenceParams,
  ): Promise<GetManyReferenceResult<T>> => {
    // Nested AREST path: /arest/{target}/{id}/{resource}. If the worker
    // doesn't have a child-navigation link between the two nouns the
    // path resolves to 404 — callers can fall back by adjusting their
    // constraint graph.
    const url = `${baseUrl}/${params.target}/${encodeId(params.id)}/${resource}${listQuery({
      pagination: params.pagination,
      sort: params.sort,
      filter: params.filter,
    })}`
    const body = await request(url, { method: 'GET' }, fetchImpl)
    const list = normalizeList<T>(body)
    return { data: list.data, total: list.total }
  }

  const create = async <T = unknown>(
    resource: string,
    params: CreateParams<T>,
  ): Promise<CreateResult<T>> => {
    const url = `${baseUrl}/${resource}`
    const body = await request(
      url,
      { method: 'POST', body: JSON.stringify(params.data) },
      fetchImpl,
    )
    return { data: unwrap<T>(body) }
  }

  const update = async <T = unknown>(
    resource: string,
    params: UpdateParams<T>,
  ): Promise<UpdateResult<T>> => {
    const url = `${baseUrl}/${resource}/${encodeId(params.id)}`
    const body = await request(
      url,
      { method: 'PATCH', body: JSON.stringify(params.data) },
      fetchImpl,
    )
    return { data: unwrap<T>(body) }
  }

  const updateMany = async <T = unknown>(
    resource: string,
    params: UpdateManyParams<T>,
  ): Promise<UpdateManyResult> => {
    await Promise.all(
      params.ids.map((id) => update<T>(resource, { id, data: params.data })),
    )
    return { data: params.ids }
  }

  const del = async <T = unknown>(
    resource: string,
    params: DeleteParams,
  ): Promise<DeleteResult<T>> => {
    const url = `${baseUrl}/${resource}/${encodeId(params.id)}`
    const body = await request(url, { method: 'DELETE' }, fetchImpl)
    return { data: unwrap<T>(body) }
  }

  const deleteMany = async (
    resource: string,
    params: DeleteManyParams,
  ): Promise<DeleteManyResult> => {
    await Promise.all(params.ids.map((id) => del(resource, { id })))
    return { data: params.ids }
  }

  return {
    getList,
    getOne,
    getMany,
    getManyReference,
    create,
    update,
    updateMany,
    delete: del,
    deleteMany,
  }
}
