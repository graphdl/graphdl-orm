/**
 * Worker-side tests for the cell-AEAD shim (#660).
 *
 * Two flavours of coverage:
 *
 *   1. Self round-trip — `cellSeal` followed by `cellOpen` recovers
 *      the original bytes, AAD/master mismatches surface as
 *      `CellAeadError`. Pins the worker-side helper API.
 *
 *   2. Cross-tier wire format — a Rust-produced sealed envelope
 *      (locked-in hex, see `cell_aead::tests::CROSS_TIER_FIXTURE_*`)
 *      opens cleanly under the same master + address via
 *      `cell_open_wasm`. Together with the Rust fixture test that
 *      asserts Rust seal == this exact hex, the pair pins the
 *      cross-tier contract: every tier (kernel, worker, future
 *      FPGA / mobile) must produce + consume these bytes byte-for-
 *      byte. Drift in either direction trips both tests.
 */

import { describe, it, expect } from 'vitest'
import {
  cellSeal,
  cellOpen,
  CellAeadError,
  canonicalAddressBytes,
  type CellAddress,
  type TenantMasterKey,
  CELL_KEY_LEN,
  NONCE_LEN,
  TAG_LEN,
} from './cell-encryption'

// ── Helpers ────────────────────────────────────────────────────────

function masterFromByte(byte: number): TenantMasterKey {
  const bytes = new Uint8Array(CELL_KEY_LEN)
  bytes.fill(byte)
  return { _bytes: bytes }
}

function decodeHex(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error('hex string length must be even')
  const out = new Uint8Array(hex.length / 2)
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16)
  }
  return out
}

function encodeHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map(b => b.toString(16).padStart(2, '0'))
    .join('')
}

// ── Cross-tier fixture (mirror of cell_aead.rs constants) ──────────
//
// These four fields MUST stay byte-for-byte identical to the
// Rust-side `CROSS_TIER_FIXTURE_*` constants. The Rust test asserts
// `cell_seal(...) == this hex`; the worker test below asserts
// `cellOpen(this hex)` recovers the plaintext. Drift in either side
// breaks both tests — the wire format contract is locked at the
// intersection.

const CROSS_TIER_FIXTURE = {
  master: masterFromByte(0xaa),
  address: {
    scope: 'worker',
    domain: 'Order',
    cellName: 'ord-42',
    version: 1n,
  } as CellAddress,
  plaintext: new TextEncoder().encode('cross-tier round-trip payload'),
  sealedHex:
    'bb3017b93796dc709e4aad59713c2b04138686ca33f233bd4ef1ff4088d3aed5607d457afa05ed72dfec9c6482d4092ab7f25574ae1112c989',
}

// ── Tests ──────────────────────────────────────────────────────────

describe('cell-encryption (worker AEAD shim)', () => {
  describe('self round-trip', () => {
    it('cellSeal → cellOpen recovers the plaintext', async () => {
      const master = masterFromByte(0x55)
      const address: CellAddress = {
        scope: 'worker',
        domain: 'Customer',
        cellName: 'cust-1',
        version: 1,
      }
      const plaintext = new TextEncoder().encode('hello, plaintext payload')
      const sealed = await cellSeal(master, address, plaintext)
      expect(sealed.length).toBe(plaintext.length + NONCE_LEN + TAG_LEN)
      const recovered = await cellOpen(master, address, sealed)
      expect(Array.from(recovered)).toEqual(Array.from(plaintext))
    })

    it('cellSeal accepts a string plaintext and round-trips through bytes', async () => {
      const master = masterFromByte(0x66)
      const address: CellAddress = {
        scope: 'worker',
        domain: 'Order',
        cellName: 'ord-7',
        version: 0,
      }
      const sealed = await cellSeal(master, address, 'plain string body')
      const recovered = await cellOpen(master, address, sealed)
      expect(new TextDecoder().decode(recovered)).toBe('plain string body')
    })

    it('rejects truncated envelopes with CellAeadError(kind="truncated")', async () => {
      const master = masterFromByte(0x77)
      const address: CellAddress = {
        scope: 's', domain: 'd', cellName: 'n', version: 0,
      }
      const tooShort = new Uint8Array(NONCE_LEN + TAG_LEN - 1)
      await expect(cellOpen(master, address, tooShort)).rejects.toThrow(CellAeadError)
      try {
        await cellOpen(master, address, tooShort)
      } catch (e) {
        expect((e as CellAeadError).kind).toBe('truncated')
      }
    })

    it('AAD mismatch (different cellName) fails as auth error', async () => {
      const master = masterFromByte(0x88)
      const addrA: CellAddress = {
        scope: 'worker', domain: 'Order', cellName: 'ord-A', version: 1,
      }
      const addrB: CellAddress = { ...addrA, cellName: 'ord-B' }
      const sealed = await cellSeal(master, addrA, 'AAD probe')
      try {
        await cellOpen(master, addrB, sealed)
        throw new Error('expected open under different address to fail')
      } catch (e) {
        expect(e).toBeInstanceOf(CellAeadError)
        expect((e as CellAeadError).kind).toBe('auth')
      }
    })

    it('cross-tenant: master B cannot open master A\'s envelope', async () => {
      const masterA = masterFromByte(0x10)
      const masterB = masterFromByte(0x20)
      const address: CellAddress = {
        scope: 'worker', domain: 'Order', cellName: 'ord-x', version: 0,
      }
      const sealed = await cellSeal(masterA, address, 'tenant probe')
      try {
        await cellOpen(masterB, address, sealed)
        throw new Error('different master must not open the envelope')
      } catch (e) {
        expect(e).toBeInstanceOf(CellAeadError)
        expect((e as CellAeadError).kind).toBe('auth')
      }
    })

    it('nonce is fresh per seal — two seals of the same plaintext differ', async () => {
      // Without a fresh nonce per seal, an observer could detect equal
      // plaintexts at the same address — a known AEAD failure mode
      // pinned by the matching Rust test (`nonce_advances_per_seal`).
      const master = masterFromByte(0x99)
      const address: CellAddress = {
        scope: 'worker', domain: 'D', cellName: 'C', version: 0,
      }
      const s1 = await cellSeal(master, address, 'identical')
      const s2 = await cellSeal(master, address, 'identical')
      expect(encodeHex(s1)).not.toBe(encodeHex(s2))
      // Both still decrypt cleanly under the same address.
      expect(new TextDecoder().decode(await cellOpen(master, address, s1))).toBe('identical')
      expect(new TextDecoder().decode(await cellOpen(master, address, s2))).toBe('identical')
    })

    it('empty plaintext round-trips (sealed = NONCE_LEN + TAG_LEN exactly)', async () => {
      const master = masterFromByte(0xab)
      const address: CellAddress = {
        scope: 'worker', domain: 'D', cellName: 'C', version: 0,
      }
      const sealed = await cellSeal(master, address, new Uint8Array(0))
      expect(sealed.length).toBe(NONCE_LEN + TAG_LEN)
      const recovered = await cellOpen(master, address, sealed)
      expect(recovered.length).toBe(0)
    })
  })

  describe('cross-tier wire format (#660 fixture)', () => {
    // This block mirrors `cell_aead::tests::cross_tier_fixture_*`. The
    // hex is locked in at both call sites — drifting either breaks the
    // matching test on the other side. Future tiers (FPGA, mobile,
    // host CLI) MUST produce + consume the same bytes.

    it('opens the Rust-produced sealed envelope and recovers the plaintext', async () => {
      const sealed = decodeHex(CROSS_TIER_FIXTURE.sealedHex)
      const recovered = await cellOpen(
        CROSS_TIER_FIXTURE.master,
        CROSS_TIER_FIXTURE.address,
        sealed,
      )
      expect(Array.from(recovered)).toEqual(Array.from(CROSS_TIER_FIXTURE.plaintext))
    })

    it('canonical address bytes match the Rust-side encoding (length-prefixed LE)', async () => {
      // Pins the AAD shape independently of the AEAD path: any future
      // change to canonicalAddressBytes that changes the byte sequence
      // would break the cross-tier-fixture-opens test below; this
      // test breaks first with a clearer message.
      const bytes = canonicalAddressBytes(CROSS_TIER_FIXTURE.address)
      // [u32 LE 6][worker][u32 LE 5][Order][u32 LE 6][ord-42][u64 LE 1]
      const expected = new Uint8Array([
        0x06, 0x00, 0x00, 0x00,
        0x77, 0x6f, 0x72, 0x6b, 0x65, 0x72, // "worker"
        0x05, 0x00, 0x00, 0x00,
        0x4f, 0x72, 0x64, 0x65, 0x72,       // "Order"
        0x06, 0x00, 0x00, 0x00,
        0x6f, 0x72, 0x64, 0x2d, 0x34, 0x32, // "ord-42"
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // u64 LE 1
      ])
      expect(Array.from(bytes)).toEqual(Array.from(expected))
    })

    it('TS-sealed envelope still satisfies the wire-format invariants', async () => {
      // Closes the loop on the reverse direction (TS seals → Rust
      // opens) without a separate process: the TS seal goes through
      // the SAME `cell_seal_wasm` Rust shim the kernel will reach
      // for, so any envelope it produces is structurally identical
      // to a kernel-produced envelope. We re-verify the structural
      // invariants here so a regression in the shim is caught even
      // if the Rust unit tests pass on the seal side.
      const sealed = await cellSeal(
        CROSS_TIER_FIXTURE.master,
        CROSS_TIER_FIXTURE.address,
        CROSS_TIER_FIXTURE.plaintext,
      )
      // sealed = [12-byte nonce | ciphertext | 16-byte tag]
      expect(sealed.length).toBe(
        CROSS_TIER_FIXTURE.plaintext.length + NONCE_LEN + TAG_LEN,
      )
      // And open recovers the original plaintext under the same
      // (master, address) pair.
      const recovered = await cellOpen(
        CROSS_TIER_FIXTURE.master,
        CROSS_TIER_FIXTURE.address,
        sealed,
      )
      expect(Array.from(recovered)).toEqual(Array.from(CROSS_TIER_FIXTURE.plaintext))
    })
  })
})
