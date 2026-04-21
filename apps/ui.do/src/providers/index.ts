/**
 * Provider factories — call once per app with a config object and hand
 * the result to the mdxui <App /> shell.
 *
 *   const dataProvider = createArestDataProvider({ baseUrl })
 *   const authProvider = createArestAuthProvider({ baseUrl })
 *   const navProvider  = createArestNavigationProvider({ baseUrl })
 *
 * Providers read session cookies via fetch's `credentials: include`.
 * The navigation provider caches its resource list on first call;
 * tests can construct a fresh factory per run.
 */
export { createArestDataProvider, ArestError } from './arestDataProvider'
export type { ArestDataProviderOptions } from './arestDataProvider'
export { createArestAuthProvider } from './arestAuthProvider'
export type { ArestAuthProviderOptions } from './arestAuthProvider'
export { createArestNavigationProvider } from './arestNavigationProvider'
export type { ArestNavigationProviderOptions } from './arestNavigationProvider'
export type {
  ArestDataProvider,
  ArestAuthProvider,
  ArestNavigationProvider,
  ArestEnvelope,
  ArestResource,
  ArestMenuItem,
  UserIdentity,
  LoginParams,
  Identifier,
  GetListParams,
  GetListResult,
  GetOneParams,
  GetOneResult,
  GetManyParams,
  GetManyResult,
  GetManyReferenceParams,
  GetManyReferenceResult,
  CreateParams,
  CreateResult,
  UpdateParams,
  UpdateResult,
  UpdateManyParams,
  UpdateManyResult,
  DeleteParams,
  DeleteResult,
  DeleteManyParams,
  DeleteManyResult,
} from './types'
