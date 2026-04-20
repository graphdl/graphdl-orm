/**
 * createArestResource(noun, options) — generate a ResourceDefinition
 * compatible with @mdxui/admin's Admin/Resource primitives.
 *
 * The four schema-driven views (Generic{List,Show,Edit,Create}View)
 * are specialised for the noun and wrapped as zero-argument
 * ComponentTypes that Admin mounts via its generated routes:
 *   /:slug             -> list
 *   /:slug/create      -> create
 *   /:slug/:id         -> edit
 *   /:slug/:id/show    -> show
 *
 * The :id segment is read from @mdxui/admin's useCurrentResource so
 * the show / edit components work inside an AdminRouter without the
 * caller threading the id manually.
 */
import type { ComponentType, ReactNode } from 'react'
import { useCurrentResource } from '@mdxui/admin'
import type { ResourceDefinition } from './resourceDefinition'
import { nounToSlug } from '../query'
import { humanize } from '../schema/openApiSchema'
import {
  GenericCreateView,
  GenericEditView,
  GenericListView,
  GenericShowView,
} from '../views'

export interface CreateArestResourceOptions {
  /** AREST worker base URL. */
  baseUrl: string
  /** App scope for /api/openapi.json?app=<name>. Defaults to 'ui.do'. */
  app?: string
  /** Override the sidebar label; defaults to pluralised humanize(noun). */
  label?: string
  /** Sidebar icon. */
  icon?: ReactNode
  /** Hide the resource from the menu (for programmatic-only routes). */
  hideFromMenu?: boolean
}

/**
 * Build a ResourceDefinition that mounts schema-driven CRUD views
 * for the given noun.
 */
export function createArestResource(
  noun: string,
  options: CreateArestResourceOptions,
): ResourceDefinition {
  const slug = nounToSlug(noun)
  const app = options.app

  const ListComponent: ComponentType = () => (
    <GenericListView noun={noun} baseUrl={options.baseUrl} app={app} />
  )
  ListComponent.displayName = `${noun}ListView`

  const CreateComponent: ComponentType = () => (
    <GenericCreateView noun={noun} baseUrl={options.baseUrl} app={app} />
  )
  CreateComponent.displayName = `${noun}CreateView`

  const EditComponent: ComponentType = () => {
    const { recordId } = useCurrentResource()
    return <GenericEditView noun={noun} id={recordId ?? ''} baseUrl={options.baseUrl} app={app} />
  }
  EditComponent.displayName = `${noun}EditView`

  const ShowComponent: ComponentType = () => {
    const { recordId } = useCurrentResource()
    return <GenericShowView noun={noun} id={recordId ?? ''} baseUrl={options.baseUrl} app={app} />
  }
  ShowComponent.displayName = `${noun}ShowView`

  return {
    name: slug,
    list: ListComponent,
    create: CreateComponent,
    edit: EditComponent,
    show: ShowComponent,
    icon: options.icon,
    options: {
      label: options.label ?? `${humanize(noun)}s`,
      hideFromMenu: options.hideFromMenu ?? false,
    },
  }
}
