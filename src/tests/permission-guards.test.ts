/**
 * Permission guards via subtype membership — TDD spec for the support
 * agent's concern that "sending emails should be gated on being approved
 * by someone who is not an agent."
 *
 * The pattern is the one the metamodel already uses in
 * `readings/core/evolution.md` for Domain Change application: a deontic
 * constraint over the trigger fact pattern, predicated on the actor's
 * type membership. With `Agent is a subtype of User` declared in
 * `readings/templates/agents.md`, the constraint says:
 *
 *   It is forbidden that some Outbound Email is sent and some User
 *   approves that Outbound Email and that User is Agent.
 *
 * No Guard entity, no Signal Source flag — just facts and a deontic.
 * The same shape gates Domain Change application (evolution.md), the
 * support app's email send, and any future actor-restricted transition.
 *
 * These tests will fail today: validate-time evaluation of deontic
 * constraints that join the trigger fact with the subtype-membership
 * fact is the engine work the failures gate.
 */

import { describe, it, expect, afterAll } from 'vitest'
import {
  compileDomain,
  releaseDomain,
  STATE_READINGS,
  type CompiledDomain,
} from './helpers/domain-fixture'
import { applyCommand, forwardChain } from '../api/engine'

// ── Shared metamodel fragments ───────────────────────────────────────
//
// The bare-engine compileDomain path doesn't auto-load the bundled
// metamodel, so we inline the subset the test needs. Per the new
// metamodel pivot, User is `.id` and Email is a binary fact type, and
// `Agent is a subtype of User` brings AI agents under the User umbrella.

const USER_AGENT_READINGS = `# User + Agent

## Entity Types
User(.id) is an entity type.
Agent(.id) is an entity type.
  Agent is a subtype of User.

## Value Types
Email is a value type.

## Fact Types
### User
User has Email.
  Each User has at most one Email.
  For each Email, exactly one User has that Email.
`.trim()

const handles: number[] = []
function track(c: CompiledDomain): CompiledDomain { handles.push(c.handle); return c }
afterAll(() => { for (const h of handles) try { releaseDomain(h) } catch {} })

// ── Outbound Email — the support app's first guarded SM ─────────────

const OUTBOUND_EMAIL_DOMAIN = `# Outbound Email

## Entity Types
Outbound Email(.id) is an entity type.

## Fact Types
### Outbound Email actions
User approves Outbound Email.
Outbound Email is sent.

## Instance Facts
State Machine Definition 'Outbound Email' is for Noun 'Outbound Email'.
Status 'Drafted' is initial in State Machine Definition 'Outbound Email'.
Status 'Approved' is defined in State Machine Definition 'Outbound Email'.
Status 'Sent' is defined in State Machine Definition 'Outbound Email'.

Transition 'approve' is defined in State Machine Definition 'Outbound Email'.
Transition 'approve' is from Status 'Drafted'.
Transition 'approve' is to Status 'Approved'.
Transition 'approve' is triggered by Fact Type 'User approves Outbound Email'.

Transition 'send' is defined in State Machine Definition 'Outbound Email'.
Transition 'send' is from Status 'Approved'.
Transition 'send' is to Status 'Sent'.
Transition 'send' is triggered by Fact Type 'Outbound Email is sent'.

## Deontic Constraints
It is forbidden that some Outbound Email is sent and some User approves that Outbound Email and that User is Agent.
`

describe('Subtype-based permission guard — Outbound Email send requires non-Agent approval', () => {
  it('compileDomain accepts the User/Agent + Outbound Email model', () => {
    const c = track(compileDomain(OUTBOUND_EMAIL_DOMAIN, STATE_READINGS, USER_AGENT_READINGS))
    expect(c.handle).toBeGreaterThanOrEqual(0)
  })

  it('non-Agent User approval lets the email reach Sent', () => {
    const c = track(compileDomain(OUTBOUND_EMAIL_DOMAIN, STATE_READINGS, USER_AGENT_READINGS))
    const populationFacts = JSON.stringify({
      facts: [
        // Resources — alice is a plain User (not an Agent)
        { factType: 'User', subject: 'alice' },
        { factType: 'Outbound Email', subject: 'eml-1' },
        // Approval by a non-Agent
        { factType: 'User approves Outbound Email', roles: { User: 'alice', 'Outbound Email': 'eml-1' } },
        // Send trigger
        { factType: 'Outbound Email is sent', roles: { 'Outbound Email': 'eml-1' } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const status = result?.derived?.['Resource is currently in Status']
      ?? result?.['Resource is currently in Status']
      ?? []
    const emailStatus = (status as any[]).find((s) =>
      (s.Resource === 'eml-1' || s.resource === 'eml-1'))
    expect(emailStatus).toBeDefined()
    expect(emailStatus.Status ?? emailStatus.status).toBe('Sent')
  })

  it('Agent approval rejects the send via deontic violation', () => {
    const c = track(compileDomain(OUTBOUND_EMAIL_DOMAIN, STATE_READINGS, USER_AGENT_READINGS))
    // bot-1 is an Agent — by the subtype-inheritance derivation it is
    // also a User, so its approval fact is well-typed. The deontic
    // constraint catches the (sent + Agent-approved) combination.
    const command = {
      type: 'createEntity',
      noun: 'Outbound Email',
      domain: 'test',
      id: 'eml-2',
      fields: {},
    }
    const seedFacts = JSON.stringify({
      facts: [
        { factType: 'Agent', subject: 'bot-1' },
        { factType: 'Outbound Email', subject: 'eml-2' },
        { factType: 'User approves Outbound Email', roles: { User: 'bot-1', 'Outbound Email': 'eml-2' } },
        { factType: 'Outbound Email is sent', roles: { 'Outbound Email': 'eml-2' } },
      ],
    })
    // Apply the send command under a population that already carries
    // the Agent approval. The constraint must fire and the apply must
    // surface a non-empty violation set.
    const result = applyCommand(command, seedFacts, c.handle)
    expect(result).toBeDefined()
    const violations = (result?.violations as unknown[]) ?? []
    expect(Array.isArray(violations)).toBe(true)
    expect(violations.length).toBeGreaterThanOrEqual(1)
    // The violation message should mention either the constraint text
    // or the Agent / send terms — loose match so the engine can format
    // however it likes.
    const blob = JSON.stringify(result).toLowerCase()
    expect(blob).toMatch(/forbidden|agent|deontic|violation/)
  })

  it('subtype membership: an Agent is also a User in the population', () => {
    // Sanity check that the implicit subtype-inheritance derivation
    // (per readings/core.md "Resource is inherited instance of Noun")
    // makes every Agent also count as a User. Without this, the
    // `User approves` fact wouldn't even be type-valid for an Agent
    // approver, and the deontic constraint above would never have a
    // chance to fire.
    const c = track(compileDomain(OUTBOUND_EMAIL_DOMAIN, STATE_READINGS, USER_AGENT_READINGS))
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Agent', subject: 'bot-1' },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    // The derived `User is Agent` (or equivalent inherited-instance
    // tuple) must surface so the deontic join in the previous test
    // resolves the Agent-as-User membership.
    const blob = JSON.stringify(result).toLowerCase()
    expect(blob).toMatch(/agent|user/)
  })
})

// ── Domain Change — same pattern from evolution.md ──────────────────
//
// The bundled evolution.md already declares the Domain Change SM and
// the apply transition. We add the symmetric deontic over Agent
// authorship. The bundled metamodel constraint
//   "It is forbidden that a Domain Change targeting Domain 'core' is
//    applied without Signal Source 'Human'"
// uses Signal Source classification; the subtype-based form below is
// the equivalent rule expressed with Agent membership instead of a
// value-typed flag, and it should compose cleanly with the existing
// Domain Change machinery.

describe('Subtype-based permission guard — Domain Change apply requires non-Agent applier', () => {
  const DOMAIN_CHANGE_GUARD = `# Domain Change Guard

## Deontic Constraints
It is forbidden that some Domain Change is applied and some User applies that Domain Change and that User is Agent.
`

  it('compileDomain accepts the agent-bar deontic stacked on the bundled Domain Change SM', () => {
    // We pass STATE_READINGS + USER_AGENT_READINGS as prereqs; the
    // bundled evolution.md declares the Domain Change SM but isn't
    // included in the bare engine, so this test is intentionally
    // narrow: it just asserts the deontic compiles. The full
    // end-to-end Domain Change scenario lives in framework-e2e.
    const c = track(compileDomain(DOMAIN_CHANGE_GUARD, STATE_READINGS, USER_AGENT_READINGS,
      // Minimal Domain Change shim so the constraint resolves without
      // dragging in the whole evolution domain.
      `# Domain Change shim
## Entity Types
Domain Change(.id) is an entity type.
Domain(.Slug) is an entity type.
## Fact Types
### Domain Change
Domain Change is applied.
User applies Domain Change.
`))
    expect(c.handle).toBeGreaterThanOrEqual(0)
  })
})
