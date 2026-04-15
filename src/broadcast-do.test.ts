import { describe, it, expect } from 'vitest'
import {
  createRegistry,
  subscribe,
  unsubscribe,
  listSubscribers,
  matches,
  publish,
  formatSseFrame,
  openSseStream,
  type CellEvent,
  type SubscriptionFilter,
} from './broadcast-do'

function makeEvent(partial: Partial<CellEvent> = {}): Omit<CellEvent, 'sequence'> {
  return {
    domain: 'organizations',
    noun: 'Organization',
    entityId: 'org-1',
    operation: 'create',
    facts: {},
    timestamp: 0,
    ...partial,
  }
}

describe('BroadcastDO registry', () => {
  describe('subscribe / unsubscribe', () => {
    it('assigns distinct ids across subscribe calls', () => {
      const reg = createRegistry()
      const id1 = subscribe(reg, { domain: 'x' }, () => {})
      const id2 = subscribe(reg, { domain: 'x' }, () => {})
      expect(id1).not.toBe(id2)
      expect(listSubscribers(reg)).toEqual([id1, id2])
    })

    it('unsubscribe removes and is idempotent', () => {
      const reg = createRegistry()
      const id = subscribe(reg, { domain: 'x' }, () => {})
      expect(unsubscribe(reg, id)).toBe(true)
      expect(unsubscribe(reg, id)).toBe(false)
      expect(listSubscribers(reg)).toEqual([])
    })

    it('requires a domain on the filter', () => {
      const reg = createRegistry()
      expect(() => subscribe(reg, {} as SubscriptionFilter, () => {})).toThrow()
    })

    it('lists subscribers in insertion order', () => {
      const reg = createRegistry()
      const a = subscribe(reg, { domain: 'x' }, () => {})
      const b = subscribe(reg, { domain: 'y' }, () => {})
      unsubscribe(reg, a)
      const c = subscribe(reg, { domain: 'z' }, () => {})
      expect(listSubscribers(reg)).toEqual([b, c])
    })
  })

  describe('matches — filter semantics', () => {
    const evt: CellEvent = { ...makeEvent(), sequence: 0 }

    it('matches when every declared field matches', () => {
      expect(matches({ domain: 'organizations' }, evt)).toBe(true)
      expect(matches({ domain: 'organizations', noun: 'Organization' }, evt)).toBe(true)
      expect(matches({
        domain: 'organizations', noun: 'Organization', entityId: 'org-1',
      }, evt)).toBe(true)
    })

    it('rejects when any declared field mismatches', () => {
      expect(matches({ domain: 'other' }, evt)).toBe(false)
      expect(matches({ domain: 'organizations', noun: 'User' }, evt)).toBe(false)
      expect(matches({
        domain: 'organizations', noun: 'Organization', entityId: 'org-2',
      }, evt)).toBe(false)
    })

    it('treats omitted fields as wildcards', () => {
      const usersEvt: CellEvent = { ...makeEvent({ noun: 'User' }), sequence: 0 }
      // Subscriber omitted noun → matches events for ANY noun in the domain
      expect(matches({ domain: 'organizations' }, evt)).toBe(true)
      expect(matches({ domain: 'organizations' }, usersEvt)).toBe(true)
    })
  })

  describe('publish — fanout', () => {
    it('assigns monotonically-increasing sequence numbers', () => {
      const reg = createRegistry()
      const a = publish(reg, makeEvent())
      const b = publish(reg, makeEvent())
      expect(a.sequence).toBe(0)
      expect(b.sequence).toBe(1)
    })

    it('delivers to every matching subscriber once', () => {
      const reg = createRegistry()
      const received: string[] = []
      subscribe(reg, { domain: 'organizations' }, e => received.push(`a:${e.entityId}`))
      subscribe(reg, { domain: 'organizations', noun: 'Organization' },
        e => received.push(`b:${e.entityId}`))
      subscribe(reg, { domain: 'other' }, e => received.push(`c:${e.entityId}`))

      publish(reg, makeEvent({ entityId: 'org-1' }))
      publish(reg, makeEvent({ entityId: 'org-2' }))

      expect(received).toEqual(['a:org-1', 'b:org-1', 'a:org-2', 'b:org-2'])
    })

    it('does not deliver to non-matching subscribers', () => {
      const reg = createRegistry()
      let hits = 0
      subscribe(reg, { domain: 'other-domain' }, () => { hits++ })
      publish(reg, makeEvent())
      expect(hits).toBe(0)
    })

    it('isolates throwing subscribers — fanout continues', () => {
      const reg = createRegistry()
      const received: string[] = []
      subscribe(reg, { domain: 'organizations' }, () => { throw new Error('bad') })
      subscribe(reg, { domain: 'organizations' }, e => received.push(e.entityId))

      expect(() => publish(reg, makeEvent())).not.toThrow()
      expect(received).toEqual(['org-1'])
    })

    it('respects entityId filter for narrow subscriptions', () => {
      const reg = createRegistry()
      const received: string[] = []
      subscribe(reg,
        { domain: 'organizations', noun: 'Organization', entityId: 'org-42' },
        e => received.push(e.entityId),
      )

      publish(reg, makeEvent({ entityId: 'org-1' }))
      publish(reg, makeEvent({ entityId: 'org-42' }))
      publish(reg, makeEvent({ entityId: 'org-7' }))

      expect(received).toEqual(['org-42'])
    })

    it('returns the published event with its sequence', () => {
      const reg = createRegistry()
      const out = publish(reg, makeEvent({ operation: 'transition' }))
      expect(out.sequence).toBe(0)
      expect(out.operation).toBe('transition')
      expect(out.entityId).toBe('org-1')
    })

    it('removed subscribers stop receiving events', () => {
      const reg = createRegistry()
      const received: number[] = []
      const id = subscribe(reg, { domain: 'organizations' }, e => received.push(e.sequence))
      publish(reg, makeEvent())
      unsubscribe(reg, id)
      publish(reg, makeEvent())
      expect(received).toEqual([0])
    })
  })

  describe('openSseStream — end-to-end smoke', () => {
    it('returns 400 without a domain query param', async () => {
      const reg = createRegistry()
      const req = new Request('https://do/events')
      const res = await openSseStream(reg, req)
      expect(res.status).toBe(400)
    })

    it('returns text/event-stream with a connected comment and deliver published events', async () => {
      const reg = createRegistry()
      const req = new Request('https://do/events?domain=organizations&noun=Organization')
      const res = await openSseStream(reg, req)

      expect(res.status).toBe(200)
      expect(res.headers.get('Content-Type')).toBe('text/event-stream')
      expect(res.headers.get('Cache-Control')).toBe('no-cache, no-transform')
      expect(res.headers.get('Connection')).toBe('keep-alive')
      expect(res.headers.get('X-Accel-Buffering')).toBe('no')

      // The registry now has one subscription for the filter.
      expect(listSubscribers(reg)).toHaveLength(1)

      // Publish a matching event and read the next frame from the body.
      publish(reg, {
        domain: 'organizations',
        noun: 'Organization',
        entityId: 'org-1',
        operation: 'create',
        facts: { name: 'Acme' },
        timestamp: 1000,
      })

      const reader = res.body!.getReader()
      const decoder = new TextDecoder()
      let buffer = ''
      // Read until we accumulate both the connected-comment and the
      // first data frame. SSE frames are delimited by "\n\n".
      while (!buffer.includes('data: ')) {
        const { value, done } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
      }
      await reader.cancel().catch(() => {})

      expect(buffer).toMatch(/^: connected sub=/m)
      expect(buffer).toMatch(/data: \{.*"entityId":"org-1"/)
      expect(buffer).toMatch(/"operation":"create"/)
    })

    it('skips non-matching events per the subscription filter', async () => {
      const reg = createRegistry()
      const req = new Request('https://do/events?domain=organizations&entityId=org-42')
      const res = await openSseStream(reg, req)

      // Publish events with different entityIds; only org-42 matches.
      publish(reg, { domain: 'organizations', noun: 'Organization', entityId: 'org-1',  operation: 'create', facts: {}, timestamp: 0 })
      publish(reg, { domain: 'organizations', noun: 'Organization', entityId: 'org-42', operation: 'create', facts: { hit: true }, timestamp: 0 })
      publish(reg, { domain: 'organizations', noun: 'Organization', entityId: 'org-7',  operation: 'create', facts: {}, timestamp: 0 })

      const reader = res.body!.getReader()
      const decoder = new TextDecoder()
      let buffer = ''
      while (!buffer.includes('"hit":true')) {
        const { value, done } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
      }
      await reader.cancel().catch(() => {})

      // Only the org-42 frame appears in the stream.
      expect(buffer).toContain('"entityId":"org-42"')
      expect(buffer).not.toContain('"entityId":"org-1"')
      expect(buffer).not.toContain('"entityId":"org-7"')
    })

    it('unsubscribes when the request signal aborts', async () => {
      const reg = createRegistry()
      const controller = new AbortController()
      const req = new Request('https://do/events?domain=organizations', { signal: controller.signal })
      await openSseStream(reg, req)
      expect(listSubscribers(reg)).toHaveLength(1)

      controller.abort()
      // The abort handler runs synchronously on dispatch.
      expect(listSubscribers(reg)).toHaveLength(0)
    })

    // ── Cross-origin SSE through a proxy (apis catch-all) ─────────
    //
    // The apis worker wraps every /arest/* response:
    //   const wrapped = new Response(response.body, response)
    //   for (const [k, v] of Object.entries(cors)) wrapped.headers.set(k, v)
    // For an SSE body (a ReadableStream), the wrapping must preserve
    // the stream — if the body were consumed by the wrapper, events
    // would never reach the browser. This test simulates that proxy
    // wrapping against a real openSseStream response and verifies
    // data frames still arrive after published events.
    it('SSE response body survives proxy-style Response wrapping with CORS headers', async () => {
      const reg = createRegistry()
      const req = new Request('https://arest/events?domain=organizations')
      const res = await openSseStream(reg, req)

      // Simulate the apis catch-all: wrap the streaming response and
      // set credentialed-CORS headers (per apis/index.ts corsHeaders).
      const wrapped = new Response(res.body, res)
      wrapped.headers.set('Access-Control-Allow-Origin', 'https://ui.auto.dev')
      wrapped.headers.set('Access-Control-Allow-Credentials', 'true')

      expect(wrapped.headers.get('Content-Type')).toBe('text/event-stream')
      expect(wrapped.headers.get('Access-Control-Allow-Origin')).toBe('https://ui.auto.dev')
      expect(wrapped.headers.get('Access-Control-Allow-Credentials')).toBe('true')

      // Publish after the wrap; the stream under the wrapped response
      // must still receive events.
      publish(reg, {
        domain: 'organizations',
        noun: 'Organization',
        entityId: 'proxied-1',
        operation: 'create',
        facts: {},
        timestamp: 0,
      })

      const reader = wrapped.body!.getReader()
      const decoder = new TextDecoder()
      let buffer = ''
      while (!buffer.includes('data: ')) {
        const { value, done } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
      }
      await reader.cancel().catch(() => {})
      expect(buffer).toContain('"entityId":"proxied-1"')
    })
  })

  describe('formatSseFrame', () => {
    it('emits a single data frame with trailing blank line per SSE spec', () => {
      const evt: CellEvent = { ...makeEvent(), sequence: 7 }
      const frame = formatSseFrame(evt)
      expect(frame.startsWith('data: ')).toBe(true)
      expect(frame.endsWith('\n\n')).toBe(true)
      // Single data line — no multi-line splits on newlines in JSON
      const dataLines = frame.split('\n').filter(l => l.startsWith('data:'))
      expect(dataLines).toHaveLength(1)
    })

    it('encodes the CellEvent as JSON in the data field', () => {
      const evt: CellEvent = { ...makeEvent({ entityId: 'sherlock' }), sequence: 2 }
      const frame = formatSseFrame(evt)
      const jsonPart = frame.replace(/^data: /, '').replace(/\n\n$/, '')
      const parsed = JSON.parse(jsonPart)
      expect(parsed.entityId).toBe('sherlock')
      expect(parsed.sequence).toBe(2)
    })
  })
})
