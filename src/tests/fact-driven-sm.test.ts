/**
 * Fact-driven state machines + event ingest — TDD spec for the metamodel
 * shape the support readings agent landed (commit 64f0494f) plus the new
 * `readings/core/ingest.md` Webhook Event Type pattern.
 *
 * These tests document the intended invariants. Some will fail today
 * because the engine has not finished wiring the new declarative model
 * end-to-end; the failures are the spec for the next round of engine
 * work, not regressions to revert.
 *
 * Both scenarios run against the in-process WASM via the `compileDomain`
 * helper (no network, no kernel boot). When you want to exercise the
 * same flows over HTTP against a running kernel/worker, use the
 * BASE-gated suites in `framework-e2e.test.ts` and `e2e-hateoas.test.ts`.
 *
 * ── Scenario 1: Transition is triggered by Fact Type ──────────────────
 *
 * Per `readings/core/state.md`:
 *
 *   Each Transition is triggered by exactly one Fact Type.
 *
 * And per `readings/core/instances.md`:
 *
 *   Fact triggered Transition for Resource.
 *   If some Fact triggered some Transition for some Resource then that
 *   Fact is of some Fact Type where that Transition is triggered by
 *   that Fact Type.
 *
 *   Resource is currently in Status. (derived from the latest Fact
 *   triggered Transition for that Resource.)
 *
 * Test: declare Order SM with `Customer places Order` as the trigger
 * for `place: In Cart -> Placed`. Add a `Customer places Order` fact
 * for Order #42 to P. Forward chain. Assert:
 *   (a) `Fact triggered Transition for Resource` derives the right
 *       triple (the place fact, the place transition, Order #42).
 *   (b) `Resource is currently in Status` for Order #42 is `Placed`.
 *
 * ── Scenario 2: Webhook Event Type yields Fact Type ───────────────────
 *
 * Per `readings/core/ingest.md`:
 *
 *   Webhook Event Type yields Fact Type with Role from JSON Path.
 *
 *   When a Webhook Event arrives carrying a Webhook Event Type, the
 *   runtime constructs one Fact per yielded Fact Type. For each Role
 *   the runtime extracts the value at the declared JSON Path. If the
 *   Role's player is an entity, find-or-upsert via the Noun's reference
 *   scheme; if a value type, use the value directly.
 *
 * Test: declare a `stripe.invoice.paid` Webhook Event Type that yields
 * one fact (`Invoice was paid by Customer`) with the Customer role
 * coming from `$.data.customer.id` and Invoice from `$.data.invoice.id`.
 * Add a Webhook Event with that Payload to P. Forward chain. Assert:
 *   (a) An Invoice resource with the extracted reference exists.
 *   (b) A Customer resource with the extracted reference exists.
 *   (c) `Invoice was paid by Customer` exists with those role players.
 */

import { describe, it, expect, afterAll } from 'vitest'
import {
  compileDomain,
  releaseDomain,
  STATE_READINGS,
  type CompiledDomain,
} from './helpers/domain-fixture'
import { applyCommand, forwardChain, system } from '../api/engine'

// ── Shared metamodel: state + ingest ─────────────────────────────────
//
// We provide both metamodel files inline so each test runs against the
// minimal vocabulary the readings need (matches what compileDomain does
// for STATE_READINGS).

const INGEST_READINGS = `# Event Ingest

## Entity Types
Webhook Event(.id) is an entity type.
Webhook Event Type(.Name) is an entity type.
Fact Type(.id) is an entity type.
Role(.Name) is an entity type.

## Value Types
JSON Path is a value type.
Payload is a value type.

## Fact Types

### Webhook Event
Webhook Event has Webhook Event Type.
Webhook Event has Payload.

### Yields
Webhook Event Type yields Fact Type with Role from JSON Path.
`.trim()

const handles: number[] = []
function track(c: CompiledDomain): CompiledDomain { handles.push(c.handle); return c }
afterAll(() => { for (const h of handles) try { releaseDomain(h) } catch {} })

// ── Scenario 1: Fact-driven SM transition ────────────────────────────

describe('Transition is triggered by Fact Type — fact-driven SM', () => {
  // Order SM: In Cart -> Placed -> Shipped -> Delivered, each step
  // triggered by a distinct Fact Type. The triggers are regular binary
  // FTs (Customer places Order, etc.) declared in the domain.
  const ORDER_SM = `# Orders

## Entity Types
Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.

## Value Types
OrderId is a value type.

## Fact Types

### Order action triggers
Customer places Order.
Customer ships Order.
Customer receives Order.

## Instance Facts
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Status 'Placed' is defined in State Machine Definition 'Order'.
Status 'Shipped' is defined in State Machine Definition 'Order'.
Status 'Delivered' is defined in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'ship' is defined in State Machine Definition 'Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Fact Type 'Customer ships Order'.
Transition 'deliver' is defined in State Machine Definition 'Order'.
Transition 'deliver' is from Status 'Shipped'.
Transition 'deliver' is to Status 'Delivered'.
Transition 'deliver' is triggered by Fact Type 'Customer receives Order'.
`

  it('compileDomain accepts the trigger declarations without error', () => {
    const c = track(compileDomain(ORDER_SM, 'orders'))
    // Compilation must not drop the SM. transitions:Order, In Cart should
    // surface at least the `place` transition once the engine wires
    // triggers into the SM extractor.
    expect(c.handle).toBeGreaterThanOrEqual(0)
  })

  it('a Fact of the trigger type derives `Fact triggered Transition for Resource`', () => {
    const c = track(compileDomain(ORDER_SM, 'orders'))
    // P seeded with one Customer + one Order resource and one trigger fact.
    const populationFacts = JSON.stringify({
      facts: [
        // Resources
        { factType: 'Customer', subject: 'alice' },
        { factType: 'Order', subject: '42' },
        // Trigger fact
        { factType: 'Customer places Order', roles: { Customer: 'alice', Order: '42' } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    // The derivation rule should yield exactly one
    //   Fact triggered Transition for Resource
    // entry binding the trigger fact, the `place` transition, Order '42'.
    const triggered = result?.derived?.['Fact triggered Transition for Resource']
      ?? result?.['Fact triggered Transition for Resource']
      ?? []
    expect(Array.isArray(triggered)).toBe(true)
    expect(triggered.length).toBeGreaterThanOrEqual(1)
    const hit = triggered.find((t: any) =>
      (t.Transition === 'place' || t.transition === 'place')
      && (t.Resource === '42' || t.resource === '42'))
    expect(hit).toBeDefined()
  })

  it('Resource is currently in Status reflects the latest triggered transition', () => {
    const c = track(compileDomain(ORDER_SM, 'orders'))
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Customer', subject: 'alice' },
        { factType: 'Order', subject: '42' },
        { factType: 'Customer places Order', roles: { Customer: 'alice', Order: '42' } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    // The SM fold should derive that Order '42' is currently in 'Placed'.
    const status = result?.derived?.['Resource is currently in Status']
      ?? result?.['Resource is currently in Status']
      ?? []
    const orderStatus = status.find((s: any) =>
      (s.Resource === '42' || s.resource === '42'))
    expect(orderStatus).toBeDefined()
    expect(orderStatus.Status ?? orderStatus.status).toBe('Placed')
  })

  it('a second trigger fact advances the SM through the chain', () => {
    const c = track(compileDomain(ORDER_SM, 'orders'))
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Customer', subject: 'alice' },
        { factType: 'Order', subject: '42' },
        { factType: 'Customer places Order', roles: { Customer: 'alice', Order: '42' } },
        { factType: 'Customer ships Order', roles: { Customer: 'alice', Order: '42' } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const status = result?.derived?.['Resource is currently in Status']
      ?? result?.['Resource is currently in Status']
      ?? []
    const orderStatus = status.find((s: any) =>
      (s.Resource === '42' || s.resource === '42'))
    expect(orderStatus).toBeDefined()
    expect(orderStatus.Status ?? orderStatus.status).toBe('Shipped')
  })

  it('apply verb routes a fact-typed command into the trigger pipeline', () => {
    const c = track(compileDomain(ORDER_SM, 'orders'))
    const empty = JSON.stringify({ facts: [
      { factType: 'Customer', subject: 'alice' },
      { factType: 'Order', subject: '42' },
    ] })
    const command = {
      factType: 'Customer places Order',
      roles: { Customer: 'alice', Order: '42' },
    }
    const result = applyCommand(command, empty, c.handle)
    // applyCommand should surface either a successful command envelope
    // (committed:true / __state populated) or an error envelope. We're
    // testing the contract that it does NOT silently swallow the fact.
    expect(result).toBeDefined()
    expect(result).not.toBeNull()
    const failed = (result?.errors?.length ?? 0) > 0
      || result?.committed === false
      || /unknown|unrecognized/i.test(JSON.stringify(result))
    expect(failed).toBe(false)
  })
})

// ── Scenario 2: Webhook event → fact materialization ─────────────────

describe('Webhook Event Type yields Fact Type — event ingest', () => {
  const STRIPE_INGEST = `${INGEST_READINGS}

## Domain: stripe-ingest

## Entity Types
Invoice(.InvoiceId) is an entity type.
Customer(.CustomerId) is an entity type.

## Value Types
InvoiceId is a value type.
CustomerId is a value type.

## Fact Types

### Invoice
Invoice was paid by Customer.

## Instance Facts
Webhook Event Type 'invoice.paid' is for Webhook Event Type 'invoice.paid'.
Webhook Event Type 'invoice.paid' yields Fact Type 'Invoice was paid by Customer' with Role 'Invoice' from JSON Path '$.data.invoice.id'.
Webhook Event Type 'invoice.paid' yields Fact Type 'Invoice was paid by Customer' with Role 'Customer' from JSON Path '$.data.customer.id'.
`

  it('compileDomain accepts the Webhook Event Type yields declarations', () => {
    const c = track(compileDomain(STRIPE_INGEST, 'stripe-ingest'))
    expect(c.handle).toBeGreaterThanOrEqual(0)
  })

  it('a Webhook Event with payload materialises the yielded Fact', () => {
    const c = track(compileDomain(STRIPE_INGEST, 'stripe-ingest'))
    // Simulate the webhook delivery — one Webhook Event resource carrying
    // a Payload that satisfies both JSON Path extractions.
    const payload = JSON.stringify({
      data: { invoice: { id: 'inv_001' }, customer: { id: 'cus_001' } },
    })
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Webhook Event', subject: 'evt_001' },
        { factType: 'Webhook Event has Webhook Event Type', roles: {
          'Webhook Event': 'evt_001',
          'Webhook Event Type': 'invoice.paid',
        } },
        { factType: 'Webhook Event has Payload', roles: {
          'Webhook Event': 'evt_001',
          'Payload': payload,
        } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    // After ingest derivation, the Invoice and Customer resources must
    // exist (find-or-upsert by reference scheme on InvoiceId / CustomerId)
    // and the binary Fact `Invoice was paid by Customer` must carry the
    // pair (inv_001, cus_001).
    const facts = result?.derived?.['Invoice was paid by Customer']
      ?? result?.['Invoice was paid by Customer']
      ?? []
    expect(Array.isArray(facts)).toBe(true)
    expect(facts.length).toBeGreaterThanOrEqual(1)
    const hit = facts.find((f: any) =>
      (f.Invoice === 'inv_001' || f.invoice === 'inv_001')
      && (f.Customer === 'cus_001' || f.customer === 'cus_001'))
    expect(hit).toBeDefined()
  })

  it('a Webhook Event with missing JSON Path values does NOT materialise partial facts', () => {
    const c = track(compileDomain(STRIPE_INGEST, 'stripe-ingest'))
    // Payload missing the customer.id; per the constraint
    // "every Role of that Fact Type appears in some Webhook Event Type
    //  yields Fact Type with Role from JSON Path", the engine must
    // refuse to materialise an incomplete fact.
    const payload = JSON.stringify({
      data: { invoice: { id: 'inv_002' } },
    })
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Webhook Event', subject: 'evt_002' },
        { factType: 'Webhook Event has Webhook Event Type', roles: {
          'Webhook Event': 'evt_002',
          'Webhook Event Type': 'invoice.paid',
        } },
        { factType: 'Webhook Event has Payload', roles: {
          'Webhook Event': 'evt_002',
          'Payload': payload,
        } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const facts = result?.derived?.['Invoice was paid by Customer']
      ?? result?.['Invoice was paid by Customer']
      ?? []
    // No fact should mention inv_002 — the missing customer.id breaks
    // the all-roles-filled invariant.
    const hit = (facts as any[]).find((f) =>
      (f.Invoice === 'inv_002' || f.invoice === 'inv_002'))
    expect(hit).toBeUndefined()
  })

  it('two Webhook Events of the same Event Type each yield independent Facts', () => {
    const c = track(compileDomain(STRIPE_INGEST, 'stripe-ingest'))
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Webhook Event', subject: 'evt_a' },
        { factType: 'Webhook Event has Webhook Event Type', roles: {
          'Webhook Event': 'evt_a',
          'Webhook Event Type': 'invoice.paid',
        } },
        { factType: 'Webhook Event has Payload', roles: {
          'Webhook Event': 'evt_a',
          'Payload': JSON.stringify({ data: { invoice: { id: 'inv_a' }, customer: { id: 'cus_x' } } }),
        } },
        { factType: 'Webhook Event', subject: 'evt_b' },
        { factType: 'Webhook Event has Webhook Event Type', roles: {
          'Webhook Event': 'evt_b',
          'Webhook Event Type': 'invoice.paid',
        } },
        { factType: 'Webhook Event has Payload', roles: {
          'Webhook Event': 'evt_b',
          'Payload': JSON.stringify({ data: { invoice: { id: 'inv_b' }, customer: { id: 'cus_y' } } }),
        } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const facts = result?.derived?.['Invoice was paid by Customer']
      ?? result?.['Invoice was paid by Customer']
      ?? []
    expect((facts as any[]).length).toBeGreaterThanOrEqual(2)
    expect((facts as any[]).some((f) => f.Invoice === 'inv_a' || f.invoice === 'inv_a')).toBe(true)
    expect((facts as any[]).some((f) => f.Invoice === 'inv_b' || f.invoice === 'inv_b')).toBe(true)
  })
})

// ── Cross-cutting: ingest → trigger → SM advance ────────────────────

describe('Webhook event triggers SM advance end-to-end', () => {
  const PAYMENT_FLOW = `${INGEST_READINGS}

## Entity Types
Order(.OrderId) is an entity type.
Customer(.CustomerId) is an entity type.

## Value Types
OrderId is a value type.
CustomerId is a value type.

## Fact Types
### Order action triggers
Customer pays for Order.

## Instance Facts
State Machine Definition 'Order' is for Noun 'Order'.
Status 'Pending' is initial in State Machine Definition 'Order'.
Status 'Paid' is defined in State Machine Definition 'Order'.
Transition 'pay' is defined in State Machine Definition 'Order'.
Transition 'pay' is from Status 'Pending'.
Transition 'pay' is to Status 'Paid'.
Transition 'pay' is triggered by Fact Type 'Customer pays for Order'.
Webhook Event Type 'payment.succeeded' yields Fact Type 'Customer pays for Order' with Role 'Customer' from JSON Path '$.customer'.
Webhook Event Type 'payment.succeeded' yields Fact Type 'Customer pays for Order' with Role 'Order' from JSON Path '$.order'.
`

  it('an inbound payment.succeeded webhook drives Order #1 from Pending to Paid', () => {
    const c = track(compileDomain(PAYMENT_FLOW, 'payment-flow'))
    const payload = JSON.stringify({ customer: 'cus_42', order: 'ord_1' })
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Order', subject: 'ord_1' },
        { factType: 'Webhook Event', subject: 'evt_pay' },
        { factType: 'Webhook Event has Webhook Event Type', roles: {
          'Webhook Event': 'evt_pay',
          'Webhook Event Type': 'payment.succeeded',
        } },
        { factType: 'Webhook Event has Payload', roles: {
          'Webhook Event': 'evt_pay',
          'Payload': payload,
        } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const status = result?.derived?.['Resource is currently in Status']
      ?? result?.['Resource is currently in Status']
      ?? []
    const orderStatus = (status as any[]).find((s) =>
      (s.Resource === 'ord_1' || s.resource === 'ord_1'))
    expect(orderStatus).toBeDefined()
    expect(orderStatus.Status ?? orderStatus.status).toBe('Paid')
  })
})

// ── Alternate readings: trigger lookup by either reading ────────────
//
// FORML 2 / ORM: a binary fact type may carry multiple readings of the
// same role pair, declared `Forward / Reverse` on a single line. They
// are aliases for one fact type, not two. The §1 paper example uses
// `Order is placed by Customer / Customer places Order` and references
// the trigger as `Customer places Order`. The engine must resolve that
// trigger reference to the FT regardless of which reading was chosen
// as the canonical id, and a fact added under either reading must drive
// the same transition.

describe('Transition trigger lookup resolves through alternate readings', () => {
  const ALT_READING_SM = `# Orders

## Entity Types
Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.

## Fact Types

### Order
Order is placed by Customer / Customer places Order.

## Instance Facts
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Status 'Placed' is defined in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
`

  it('compileDomain accepts the slash alternate reading', () => {
    const c = track(compileDomain(ALT_READING_SM, 'orders'))
    expect(c.handle).toBeGreaterThanOrEqual(0)
  })

  it('a fact added by the forward reading triggers the place transition', () => {
    const c = track(compileDomain(ALT_READING_SM, 'orders'))
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Customer', subject: 'alice' },
        { factType: 'Order', subject: '42' },
        // Forward reading — same role pair as the alternate.
        { factType: 'Order is placed by Customer', roles: { Customer: 'alice', Order: '42' } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const status = result?.derived?.['Resource is currently in Status']
      ?? result?.['Resource is currently in Status']
      ?? []
    const orderStatus = (status as any[]).find((s) =>
      (s.Resource === '42' || s.resource === '42'))
    expect(orderStatus).toBeDefined()
    expect(orderStatus.Status ?? orderStatus.status).toBe('Placed')
  })

  it('a fact added by the alternate reading triggers the same transition', () => {
    const c = track(compileDomain(ALT_READING_SM, 'orders'))
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Customer', subject: 'alice' },
        { factType: 'Order', subject: '42' },
        // Alternate reading — must resolve to the same FT.
        { factType: 'Customer places Order', roles: { Customer: 'alice', Order: '42' } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const status = result?.derived?.['Resource is currently in Status']
      ?? result?.['Resource is currently in Status']
      ?? []
    const orderStatus = (status as any[]).find((s) =>
      (s.Resource === '42' || s.resource === '42'))
    expect(orderStatus).toBeDefined()
    expect(orderStatus.Status ?? orderStatus.status).toBe('Placed')
  })

  it('the trigger reference resolves regardless of which reading is canonical', () => {
    // Mirror domain — same FT, but the trigger references the FORWARD
    // reading instead of the alternate. Both should resolve.
    const MIRROR_SM = ALT_READING_SM.replace(
      "Transition 'place' is triggered by Fact Type 'Customer places Order'.",
      "Transition 'place' is triggered by Fact Type 'Order is placed by Customer'.",
    )
    const c = track(compileDomain(MIRROR_SM, 'orders'))
    const populationFacts = JSON.stringify({
      facts: [
        { factType: 'Customer', subject: 'alice' },
        { factType: 'Order', subject: '42' },
        // Use the alternate reading to commit the fact — different from
        // the trigger reference, but same FT.
        { factType: 'Customer places Order', roles: { Customer: 'alice', Order: '42' } },
      ],
    })
    const result = forwardChain(populationFacts, c.handle)
    const status = result?.derived?.['Resource is currently in Status']
      ?? result?.['Resource is currently in Status']
      ?? []
    const orderStatus = (status as any[]).find((s) =>
      (s.Resource === '42' || s.resource === '42'))
    expect(orderStatus).toBeDefined()
    expect(orderStatus.Status ?? orderStatus.status).toBe('Placed')
  })
})
