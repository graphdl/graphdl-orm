/**
 * GenericEditView — schema-driven edit form for any AREST entity.
 *
 * Wraps @mdxui/admin's <EditView>. Fields come from the noun's
 * JSON Schema; initial values from GET /arest/{slug}/{id}. Submit
 * PATCHes back through arestDataProvider.update and invalidates
 * list + one query keys on success.
 */
import { useEffect, useState, type FormEvent, type ReactElement } from 'react'
import { EditView } from '@mdxui/admin'
import { useArestOne, useArestUpdate } from '../hooks/useArestResource'
import { useOpenApiSchema, type FieldDef } from '../schema'
import { humanize } from '../schema/openApiSchema'
import { SchemaInput } from './schemaInputs'

export interface GenericEditViewProps {
  noun: string
  id: string
  baseUrl: string
  app?: string
  title?: string
  onSaved?: (next: Record<string, unknown>) => void
  onCancel?: () => void
}

export function GenericEditView(props: GenericEditViewProps): ReactElement {
  const { noun, id, baseUrl, app, title, onSaved, onCancel } = props
  const entity = useArestOne<Record<string, unknown>>(noun, id, { baseUrl })
  const schema = useOpenApiSchema(noun, { baseUrl, app })
  const mutate = useArestUpdate<Record<string, unknown>>(noun, id, { baseUrl })
  const [values, setValues] = useState<Record<string, unknown>>({})
  const [error, setError] = useState<string | null>(null)

  // Hydrate the form once the record loads. Schema fields only (so we
  // don't silently persist worker-emitted derived fields).
  useEffect(() => {
    if (!entity.data?.data || !schema.fields.length) return
    const initial: Record<string, unknown> = {}
    for (const f of schema.fields) {
      initial[f.name] = (entity.data.data as Record<string, unknown>)[f.name]
    }
    setValues(initial)
  }, [entity.data, schema.fields])

  const onSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    setError(null)
    try {
      const res = await mutate.update(values)
      onSaved?.(res.data as Record<string, unknown>)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const isLoading = entity.isLoading || schema.isLoading
  const resolvedTitle = title ?? `Edit ${humanize(noun)}: ${id}`

  return (
    <EditView
      title={resolvedTitle}
      loading={isLoading}
      record={entity.data?.data as Record<string, unknown> | undefined}
    >
      <form data-testid="generic-edit-form" onSubmit={onSubmit}>
        {schema.fields.map((f: FieldDef) => (
          <div key={f.name} style={{ marginBottom: '1rem' }}>
            <label style={{ display: 'block', fontWeight: 600 }}>{f.label}</label>
            <SchemaInput
              field={f}
              value={values[f.name]}
              onChange={(next) => setValues((prev) => ({ ...prev, [f.name]: next }))}
            />
          </div>
        ))}
        {error && <p role="alert" data-testid="edit-error" style={{ color: 'crimson' }}>{error}</p>}
        <div style={{ display: 'flex', gap: '0.5rem' }}>
          <button type="submit" disabled={mutate.isPending}>
            {mutate.isPending ? 'Saving…' : 'Save'}
          </button>
          {onCancel && (
            <button type="button" onClick={onCancel}>Cancel</button>
          )}
        </div>
      </form>
    </EditView>
  )
}

GenericEditView.displayName = 'GenericEditView'
