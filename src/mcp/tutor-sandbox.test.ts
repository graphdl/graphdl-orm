/**
 * Tutor sandbox module — unit tests.
 *
 * The sandbox is a second engine handle compiled from tutor/domains/.
 * Mutations through tutorSystemCall must not be visible to the active
 * app's handle, and vice versa.
 */
import { describe, it, expect, beforeEach } from 'vitest'
import { getSandboxHandle, tutorSystemCall, resetSandbox, tutorDomainsDir } from './tutor-sandbox.js'
import { compileDomainReadings, system } from '../api/engine.js'
import { existsSync } from 'fs'
import { resolve } from 'path'

describe('tutor sandbox — WASM mode', () => {
  beforeEach(async () => {
    await resetSandbox()
  })

  it('exposes the bundled tutor/domains directory', () => {
    const dir = tutorDomainsDir()
    expect(existsSync(dir)).toBe(true)
    expect(existsSync(resolve(dir, 'orders.md'))).toBe(true)
  })

  it('boots from tutor/domains/ on first call and exposes its noun catalog', async () => {
    const raw = await tutorSystemCall('list:Noun', '')
    const list = JSON.parse(raw)
    const names = list.map((n: any) => n.id ?? n.Name ?? n.name)
    expect(names).toContain('Order')
    expect(names).toContain('Customer')
  })

  it('isolates sandbox writes from a sibling local handle, in both directions', async () => {
    // Sandbox → local: write a Customer through the sandbox.
    await tutorSystemCall('create:Customer', '<<Name, alice-sandbox>>')

    // Build an unrelated active-app handle compiled from a tiny fixture
    // that also declares Customer.
    const localHandle = compileDomainReadings(
      'Customer(.Name) is an entity type.\nCustomer has Name.\n  Each Customer has exactly one Name.'
    )
    const localList = JSON.parse(system(localHandle, 'list:Customer', ''))
    expect(localList.find((c: any) => (c.id ?? c.Name) === 'alice-sandbox')).toBeUndefined()

    // Local → sandbox: write a Customer to the local handle and confirm
    // the sandbox cannot see it.
    system(localHandle, 'create:Customer', '<<Name, bob-local>>')
    const sandList = JSON.parse(await tutorSystemCall('list:Customer', ''))
    expect(sandList.find((c: any) => (c.id ?? c.Name) === 'bob-local')).toBeUndefined()
    // Sandbox still sees its own write.
    expect(sandList.find((c: any) => (c.id ?? c.Name) === 'alice-sandbox')).toBeDefined()
  })

  it('returns the same numeric handle across calls until reset', async () => {
    const a = await getSandboxHandle()
    const b = await getSandboxHandle()
    expect(a).toBe(b)
    expect(a).toBeGreaterThanOrEqual(0)
  })
})
