/**
 * Provider type shapes.
 *
 * These intentionally mirror @mdxui/app's simpler 6-method DataProvider
 * but extend it with the four extra methods the task (#122) called out
 * — getManyReference, updateMany, deleteMany — so the provider can also
 * slot into the wider react-admin-style interface that @mdxui/admin
 * components consume through adapters.
 *
 * Keeping the shape self-contained here means tests don't have to
 * import from @mdxui; they just assert against the local type. The
 * provider instances returned from arestDataProvider() are assignable
 * to @mdxui/app's DataProvider (structural subtype).
 */

export type Identifier = string | number

export interface GetListParams {
  pagination?: { page: number; perPage: number }
  sort?: { field: string; order: 'ASC' | 'DESC' }
  filter?: Record<string, unknown>
}

export interface GetListResult<T> {
  data: T[]
  total?: number
  pageInfo?: { hasNextPage?: boolean; hasPreviousPage?: boolean }
}

export interface GetOneParams { id: Identifier }
export interface GetOneResult<T> { data: T }

export interface GetManyParams { ids: Identifier[] }
export interface GetManyResult<T> { data: T[] }

export interface GetManyReferenceParams {
  target: string
  id: Identifier
  pagination?: { page: number; perPage: number }
  sort?: { field: string; order: 'ASC' | 'DESC' }
  filter?: Record<string, unknown>
}
export interface GetManyReferenceResult<T> {
  data: T[]
  total?: number
}

export interface CreateParams<T> { data: Partial<T> }
export interface CreateResult<T> { data: T }

export interface UpdateParams<T> {
  id: Identifier
  data: Partial<T>
  previousData?: T
}
export interface UpdateResult<T> { data: T }

export interface UpdateManyParams<T> {
  ids: Identifier[]
  data: Partial<T>
}
export interface UpdateManyResult { data: Identifier[] }

export interface DeleteParams {
  id: Identifier
  previousData?: unknown
}
export interface DeleteResult<T> { data: T }

export interface DeleteManyParams { ids: Identifier[] }
export interface DeleteManyResult { data: Identifier[] }

/**
 * Data provider surface — CRUD + bulk + reference, all resource-scoped.
 *
 * `resource` is always a **plural slug** per AREST's nounToSlug
 * convention (e.g. 'support-requests', 'organizations'). The provider
 * maps that directly onto `/arest/{resource}` on the worker.
 */
export interface ArestDataProvider {
  getList: <T = unknown>(resource: string, params?: GetListParams) => Promise<GetListResult<T>>
  getOne: <T = unknown>(resource: string, params: GetOneParams) => Promise<GetOneResult<T>>
  getMany: <T = unknown>(resource: string, params: GetManyParams) => Promise<GetManyResult<T>>
  getManyReference: <T = unknown>(resource: string, params: GetManyReferenceParams) => Promise<GetManyReferenceResult<T>>
  create: <T = unknown>(resource: string, params: CreateParams<T>) => Promise<CreateResult<T>>
  update: <T = unknown>(resource: string, params: UpdateParams<T>) => Promise<UpdateResult<T>>
  updateMany: <T = unknown>(resource: string, params: UpdateManyParams<T>) => Promise<UpdateManyResult>
  delete: <T = unknown>(resource: string, params: DeleteParams) => Promise<DeleteResult<T>>
  deleteMany: (resource: string, params: DeleteManyParams) => Promise<DeleteManyResult>
}

/**
 * Theorem-5 envelope shape returned by the AREST worker.
 *
 * Matches src/api/envelope.ts in the Rust-adjacent worker code:
 *   { data, derived?, violations?, _links }
 *
 * The data provider unwraps `data` for callers; `_links` is carried
 * through on list results so HATEOAS navigation remains available.
 */
export interface ArestEnvelope<T> {
  data: T
  derived?: Record<string, unknown>
  violations?: ReadonlyArray<{
    reading: string
    constraintId: string
    modality: 'alethic' | 'deontic'
    detail?: string
  }>
  _links?: {
    transitions?: ReadonlyArray<{ event: string; href: string; method: 'POST' }>
    navigation?: Record<string, string>
  }
}

// ── Auth types ─────────────────────────────────────────────────────

export interface LoginParams {
  username?: string
  password?: string
  [key: string]: unknown
}

export interface UserIdentity {
  id: string
  fullName?: string
  email?: string
  avatar?: string
  [key: string]: unknown
}

export interface ArestAuthProvider {
  login: (params: LoginParams) => Promise<void>
  logout: () => Promise<void>
  checkAuth: () => Promise<void>
  checkError: (error: Error | { status?: number; message?: string }) => Promise<void>
  getIdentity: () => Promise<UserIdentity>
  getPermissions: () => Promise<string[]>
}

// ── Navigation types ───────────────────────────────────────────────

export interface ArestResource {
  /** Plural slug used as resource identifier (matches AREST `/arest/{slug}`). */
  name: string
  /** Human-readable singular label, derived from the noun name. */
  label: string
  /** Human-readable plural label. */
  labelPlural: string
}

export interface ArestMenuItem {
  title: string
  url: string
  icon?: string
}

export interface ArestNavigationProvider {
  /** Load the resource set from the worker's OpenAPI document. */
  resources: () => Promise<ArestResource[]>
  /** Load a flat menu (title + url) from the same document. */
  menu: () => Promise<ArestMenuItem[]>
}
