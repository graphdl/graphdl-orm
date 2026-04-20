/**
 * GenericCreateView — schema-driven create form for any AREST noun.
 *
 * Wraps @mdxui/admin's <CreateView>. Fields come from the noun's
 * JSON Schema; submit POSTs through arestDataProvider.create and
 * invalidates the list key on success. 422 violation responses
 * surface as the form's error message carrying the original FORML 2
 * reading (whitepaper Corollary 1).
 */
import { useState, type FormEvent, type ReactElement } from 'react'
import { CreateView } from '@mdxui/admin'
import { useArestCreate } from '../hooks/useArestResource'
import { useOpenApiSchema, type FieldDef } from '../schema'
import { humanize } from '../schema/openApiSchema'
import { SchemaInput } from './schemaInputs'

export interface GenericCreateViewProps {
  noun: string
  baseUrl: string
  app?: string
  title?: string
  onCreated?: (record: Record<string, unknown>) => void
  onCancel?: () => void
}

export function GenericCreateView(props: GenericCreateViewProps): ReactElement {
  const { noun, baseUrl, app, title, onCreated, onCancel } = props
  const schema = useOpenApiSchema(noun, { baseUrl, app })
  const mutate = useArestCreate<Record<string, unknown>>(noun, { baseUrl })
  const [values, setValues] = useState<Record<string, unknown>>({})
  const [error, setError] = useState<string | null>(null)

  const onSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    setError(null)
    try {
      const res = await mutate.create(values)
      onCreated?.(res.data as Record<string, unknown>)
      setValues({})
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const resolvedTitle = title ?? `Create ${humanize(noun)}`

  return (
    <CreateView title={resolvedTitle}>
      <form data-testid="generic-create-form" onSubmit={onSubmit}>
        {schema.fields.map((f: FieldDef) => (
          <div key={f.name} style={{ marginBottom: '1rem' }}>
            <label style={{ display: 'block', fontWeight: 600 }}>
              {f.label}{f.required && <span aria-hidden="true"> *</span>}
            </label>
            <SchemaInput
              field={f}
              value={values[f.name]}
              onChange={(next) => setValues((prev) => ({ ...prev, [f.name]: next }))}
            />
          </div>
        ))}
        {error && <p role="alert" data-testid="create-error" style={{ color: 'crimson' }}>{error}</p>}
        <div style={{ display: 'flex', gap: '0.5rem' }}>
          <button type="submit" disabled={mutate.isPending}>
            {mutate.isPending ? 'Creating…' : `Create ${humanize(noun)}`}
          </button>
          {onCancel && (
            <button type="button" onClick={onCancel}>Cancel</button>
          )}
        </div>
      </form>
    </CreateView>
  )
}

GenericCreateView.displayName = 'GenericCreateView'
