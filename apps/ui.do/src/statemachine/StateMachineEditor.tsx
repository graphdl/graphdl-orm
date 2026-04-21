/**
 * StateMachineEditor — visual + editable surface for an AREST State
 * Machine Definition.
 *
 * Renders each Status as a card, its outgoing Transitions as labeled
 * rows inside the card, and a row-level edit / delete for each
 * transition. A composer at the bottom adds new transitions. Every
 * edit flows through the standard arestDataProvider so the SSE
 * bridge broadcasts the change to any other tab viewing the same
 * machine — the overworld menu of editing.
 *
 * The component also computes an xstate 5 machine config from the
 * current facts (via arestToXStateConfig) and surfaces it as a
 * read-only JSON block. Consumers who want to run the machine in
 * the browser can pick it up from `onConfigChange`.
 */
import { useEffect, useMemo, useState, type FormEvent, type ReactElement } from 'react'
import { createMachine } from 'xstate'
import {
  arestToXStateConfig,
  buildStatelyDeeplink,
  describeStatuses,
  findDeadCycles,
  listStatuses,
  type ArestTransition,
  type XStateConfig,
} from './xstateConfig'
import { useStateMachine } from './useStateMachine'

export interface StateMachineEditorProps {
  /** AREST State Machine Definition id. */
  smdId: string
  baseUrl: string
  /** Fired every time the config recomputes (after a successful edit). */
  onConfigChange?: (config: XStateConfig) => void
  /** When true, disables all edit controls (view-only mode). */
  readOnly?: boolean
  /**
   * When set, overlays an "active state" indicator on the named
   * status. Used to show the live state of a running instance:
   *   <StateMachineEditor smdId="Order" currentStatus={order.status} />
   */
  currentStatus?: string
}

interface EditorRow {
  mode: 'view' | 'edit' | 'add'
  draft: Partial<ArestTransition> & { id?: string }
}

export function StateMachineEditor(props: StateMachineEditorProps): ReactElement {
  const { smdId, baseUrl, onConfigChange, readOnly, currentStatus } = props
  const {
    smd,
    transitions,
    isLoading,
    addTransition,
    updateTransition,
    deleteTransition,
  } = useStateMachine(smdId, { baseUrl })

  const [editing, setEditing] = useState<Record<string, EditorRow>>({})
  const [adder, setAdder] = useState<EditorRow>({ mode: 'add', draft: {} })
  const [error, setError] = useState<string | null>(null)

  const xstateConfig = useMemo<XStateConfig | null>(() => {
    if (!smd) return null
    return arestToXStateConfig(smd, transitions)
  }, [smd, transitions])

  // Liveness warning — whitepaper Theorem 3 proof requires every
  // cycle to have an exit transition.
  const deadCycles = useMemo(() => {
    if (!smd) return []
    return findDeadCycles(smd, transitions)
  }, [smd, transitions])

  // Surface the xstate config to consumers — and validate it compiles.
  useEffect(() => {
    if (!xstateConfig) return
    try {
      // createMachine throws if the config is malformed (e.g. event
      // names reference non-existent states). Catching here means the
      // UI surfaces the error inline rather than crashing.
      createMachine(xstateConfig)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
    onConfigChange?.(xstateConfig)
  }, [xstateConfig, onConfigChange])

  if (isLoading) return <p data-testid="sm-loading">Loading state machine…</p>
  if (!smd) return <p data-testid="sm-missing">No State Machine Definition: {smdId}</p>

  const statuses = describeStatuses(smd, transitions)
  const statusNames = listStatuses(smd, transitions)

  // ── Event handlers ─────────────────────────────────────────────

  const startEdit = (t: ArestTransition) => {
    setEditing((prev) => ({ ...prev, [t.id]: { mode: 'edit', draft: { ...t } } }))
  }

  const cancelEdit = (id: string) => {
    setEditing((prev) => {
      const next = { ...prev }
      delete next[id]
      return next
    })
  }

  const saveEdit = async (id: string, form: Partial<ArestTransition>) => {
    try {
      await updateTransition(id, form)
      cancelEdit(id)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const onDelete = async (id: string) => {
    try { await deleteTransition(id) }
    catch (err) { setError(err instanceof Error ? err.message : String(err)) }
  }

  const onAdd = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    const d = adder.draft
    if (!d.id || !d.from || !d.to) {
      setError('Transition id, from, and to are required.')
      return
    }
    try {
      await addTransition({
        id: d.id,
        from: d.from,
        to: d.to,
        event: d.event,
      })
      setAdder({ mode: 'add', draft: {} })
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  return (
    <section data-testid="state-machine-editor">
      <header>
        <h2>{smd.noun} state machine</h2>
        <p>
          Initial: <code data-testid="sm-initial">{smd.initial}</code>
          {currentStatus && (
            <> · Current: <code data-testid="sm-current">{currentStatus}</code></>
          )}
        </p>
        {xstateConfig && (
          <p>
            <a
              data-testid="sm-stately-deeplink"
              href={buildStatelyDeeplink(xstateConfig)}
              target="_blank"
              rel="noopener noreferrer"
            >Open in Stately Studio ↗</a>
          </p>
        )}
      </header>

      {error && (
        <p role="alert" data-testid="sm-error" style={{ color: 'crimson' }}>{error}</p>
      )}

      {deadCycles.length > 0 && (
        <p
          role="alert"
          data-testid="sm-dead-cycle-warning"
          style={{ color: '#b45309', background: '#fef3c7', padding: '0.5rem', borderRadius: 4 }}
        >
          Liveness warning: cycle with no exit transition — {' '}
          {deadCycles.map((scc) => `[${scc.join(' → ')}]`).join(', ')}
          . Whitepaper Theorem 3 requires every cycle to have at least one exit.
        </p>
      )}

      <ol data-testid="sm-states" style={{ listStyle: 'none', padding: 0, display: 'grid', gap: '0.75rem' }}>
        {statuses.map((s) => (
          <li
            key={s.name}
            data-testid={`sm-state-${s.name}`}
            data-initial={s.isInitial ? 'true' : undefined}
            data-terminal={s.isTerminal ? 'true' : undefined}
            data-current={currentStatus === s.name ? 'true' : undefined}
            style={{
              border: '1px solid #ccc',
              padding: '0.5rem',
              borderRadius: 6,
              // Active-state overlay: highlighted border and tinted
              // background when a live instance sits in this state.
              ...(currentStatus === s.name
                ? { borderColor: '#2563eb', borderWidth: 2, background: '#eff6ff' }
                : {}),
            }}
          >
            <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
              <strong>{s.name}</strong>
              <small style={{ color: '#666' }}>
                {currentStatus === s.name && 'current'}
                {currentStatus === s.name && (s.isInitial || s.isTerminal) && ' · '}
                {s.isInitial && 'initial'}
                {s.isInitial && s.isTerminal && ' · '}
                {s.isTerminal && 'terminal'}
              </small>
            </header>
            {s.outgoing.length === 0 ? (
              <p style={{ color: '#888', margin: '0.25rem 0' }}>(no outgoing transitions)</p>
            ) : (
              <ul style={{ listStyle: 'none', padding: 0, margin: '0.25rem 0', display: 'grid', gap: '0.25rem' }}>
                {s.outgoing.map((t) => {
                  const row = editing[t.id]
                  if (row?.mode === 'edit') {
                    return (
                      <li key={t.id} data-testid={`sm-transition-edit-${t.id}`}>
                        <TransitionForm
                          draft={row.draft}
                          statusNames={statusNames}
                          onChange={(draft) => setEditing((p) => ({ ...p, [t.id]: { mode: 'edit', draft } }))}
                          onSubmit={() => saveEdit(t.id, row.draft)}
                          onCancel={() => cancelEdit(t.id)}
                          submitLabel="Save"
                        />
                      </li>
                    )
                  }
                  return (
                    <li key={t.id} data-testid={`sm-transition-${t.id}`} style={{ display: 'flex', gap: '0.5rem', alignItems: 'center' }}>
                      <code>{t.event ?? t.id}</code> → <code>{t.to}</code>
                      {!readOnly && (
                        <>
                          <button type="button" data-testid={`sm-edit-${t.id}`} onClick={() => startEdit(t)}>Edit</button>
                          <button type="button" data-testid={`sm-delete-${t.id}`} onClick={() => onDelete(t.id)}>Delete</button>
                        </>
                      )}
                    </li>
                  )
                })}
              </ul>
            )}
          </li>
        ))}
      </ol>

      {!readOnly && (
        <section>
          <h3>Add transition</h3>
          <form data-testid="sm-add-form" onSubmit={onAdd}>
            <TransitionForm
              draft={adder.draft}
              statusNames={statusNames}
              onChange={(draft) => setAdder({ mode: 'add', draft })}
              onSubmit={() => {}} // handled by form's onSubmit
              onCancel={() => setAdder({ mode: 'add', draft: {} })}
              submitLabel="Add"
              isNew
            />
          </form>
        </section>
      )}

      {xstateConfig && (
        <details style={{ marginTop: '1rem' }}>
          <summary>xstate config (generated)</summary>
          <pre data-testid="sm-xstate-config" style={{ background: '#f6f8fa', padding: '0.5rem', borderRadius: 4 }}>
            {JSON.stringify(xstateConfig, null, 2)}
          </pre>
        </details>
      )}
    </section>
  )
}

StateMachineEditor.displayName = 'StateMachineEditor'

interface TransitionFormProps {
  draft: Partial<ArestTransition>
  statusNames: string[]
  onChange: (draft: Partial<ArestTransition>) => void
  onSubmit: () => void
  onCancel: () => void
  submitLabel: string
  isNew?: boolean
}

function TransitionForm({
  draft, statusNames, onChange, onSubmit, onCancel, submitLabel, isNew,
}: TransitionFormProps): ReactElement {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '0.25rem 0.5rem', alignItems: 'baseline' }}>
      <label>Id</label>
      <input
        data-testid="sm-input-id"
        value={draft.id ?? ''}
        onChange={(e) => onChange({ ...draft, id: e.target.value })}
        readOnly={!isNew}
        required
      />

      <label>Event</label>
      <input
        data-testid="sm-input-event"
        placeholder="(defaults to id)"
        value={draft.event ?? ''}
        onChange={(e) => onChange({ ...draft, event: e.target.value || undefined })}
      />

      <label>From</label>
      <SelectOrNew
        testId="sm-input-from"
        value={draft.from ?? ''}
        options={statusNames}
        onChange={(v) => onChange({ ...draft, from: v })}
      />

      <label>To</label>
      <SelectOrNew
        testId="sm-input-to"
        value={draft.to ?? ''}
        options={statusNames}
        onChange={(v) => onChange({ ...draft, to: v })}
      />

      <span />
      <div style={{ display: 'flex', gap: '0.5rem' }}>
        <button type={isNew ? 'submit' : 'button'} onClick={isNew ? undefined : onSubmit} data-testid="sm-form-submit">
          {submitLabel}
        </button>
        <button type="button" onClick={onCancel} data-testid="sm-form-cancel">Cancel</button>
      </div>
    </div>
  )
}

/**
 * Select from existing statuses, or type a new one (datalist keeps
 * the friction low — a new status is just a transition whose from /
 * to references a status name not yet present, which matches
 * AREST's derived-status semantics).
 */
function SelectOrNew(props: { testId: string; value: string; options: string[]; onChange: (v: string) => void }): ReactElement {
  const listId = `${props.testId}-list`
  return (
    <>
      <input
        data-testid={props.testId}
        list={listId}
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
      />
      <datalist id={listId}>
        {props.options.map((o) => <option key={o} value={o} />)}
      </datalist>
    </>
  )
}
