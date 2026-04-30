# Tenant master key rotation (#662)

This is the operator runbook for rotating an AREST deployment's per-tenant
master key from master A → master B without losing access to existing
sealed cells.

The rotation primitive is `arest::cell_aead::rotate_cell` /
`rotate_tenant`; the SystemVerb that drives it from the engine boundary
is `system(handle, "rotate_tenant_master:<tenant_id>", <JSON body>)`,
gated behind `RegisterMode::Privileged` (same boundary as `register:`
and `load_reading:` per #328's pattern).

## What rotation actually does

Per #659 every cell crossing a serialization boundary is sealed against
a per-tenant master via:

```
sealed = [12-byte nonce | ciphertext | 16-byte Poly1305 tag]
key    = HKDF-SHA256(master, salt = address_canonical_bytes,
                     info = "arest-cell-key/v1")[..32]
```

Rotation replaces the master while leaving the cell address (scope,
domain, cell_name, version) untouched. Each cell is opened under the
old master, re-sealed under the new master with a fresh nonce, and
atomic-swapped into storage. The address bytes are unchanged so the
AEAD AAD binding is identical pre/post-rotation.

`rotate_cell` is single-cell atomic. `rotate_tenant` is the walk: it
iterates `(CellAddress, sealed_bytes)` pairs and produces a
`RotationReport`:

- `rotated`: cells that opened under the old master and re-sealed under
  the new master. The caller atomic-swaps these into storage.
- `failures`: cells the old master could NOT open (corrupted envelope,
  truncated row, etc.). These are retained under the old master
  untouched. The walk does NOT abort on the first failure — a single
  corrupt cell would otherwise hold the entire tenant hostage.

## Read-only window assumption (first-version)

Rotation is non-zero-downtime. The tenant goes read-only for the
rotation window. The caller MUST hold a per-tenant write lock for the
duration of the walk:

- **Kernel side**: the per-slot RwLock from #155 around DOMAINS, plus
  the `block_storage::MOUNT` Mutex for the on-disk checkpoint. The
  helper `arest_kernel::block_storage::rotate_checkpoint_master(old,
  new)` acquires `MOUNT` for the read-side, calls `cell_aead::
  rotate_cell`, and writes back through `checkpoint()` which
  re-acquires the same lock — read-old / write-new is atomic against
  any other writer.

- **Worker side**: each EntityDB DO is single-writer by Cloudflare's
  design (one DO instance handles requests serially). The rotation
  walk against a tenant's set of DOs is NOT single-DO, so the
  orchestrator (`worker.ts` rotation handler / RegistryDB rotation
  method) holds a per-tenant write semaphore that is mutually
  exclusive with any other tenant write for the rotation window.
  `EntityDB.rotateMaster(...)` performs the per-DO atomic swap; the
  orchestrator iterates the tenant's entity ids (via the per-tenant
  scoping from #205) and collects the per-cell results into a
  RotationReport.

Concurrent writes during rotation would observe a half-rotated cell
set: some cells readable only by old, some only by new. Either drain
the rotation or hold writes until it completes.

## Operator workflow — kernel target

1. Generate a fresh 32-byte master B (entropy source +
   `~/.arest/tenant_salt.bin`).
2. Persist B in the freeze-blob "pending" slot alongside the existing
   "active" slot. Targets that derive the master from boot entropy +
   salt store the new salt in "pending" instead of the bytes.
3. Acquire the per-tenant write lock (the per-slot RwLock from #155).
4. Call `arest_kernel::block_storage::rotate_checkpoint_master(&old,
   &new)`. The helper:
   - reads the on-disk sealed checkpoint envelope under the old master;
   - re-seals the recovered bytes under the new master;
   - writes the new envelope through the existing `checkpoint()` path
     (same CRC + header machinery, durability contract unchanged);
   - refreshes the in-memory mount cache so a subsequent
     `last_state_sealed_open(&new)` reads the just-written envelope.
5. Promote "pending" → "active" in the freeze-blob; wipe the old
   master from the boot path.
6. Release the per-tenant write lock.

There is exactly one sealed envelope on disk per kernel slot today
(the outer-envelope checkpoint). A multi-checkpoint follow-up (#666 et
al.) would extend this to the per-slot checkpoint set, at which point
the kernel would call `cell_aead::rotate_tenant` over the iterator of
sealed checkpoint envelopes.

## Operator workflow — worker target

1. `wrangler secret put TENANT_MASTER_SEED_v2 <new>` — leave the
   existing v1 slot bound during the rotation window.
2. Deploy a build that exposes both slots to the EntityDB DO via
   `env.TENANT_MASTER_SEED` and `env.TENANT_MASTER_SEED_v2`.
3. Acquire the per-tenant write semaphore at the orchestrator (the
   RegistryDB / dispatcher seam).
4. Enumerate the tenant's EntityDB cells via the per-tenant scoping
   from #205. For each cell call `EntityDB.rotateMaster({ oldSeed,
   oldSalt, newSeed, newSalt })`. The DO:
   - reads the sealed row;
   - opens under the old master;
   - re-seals under the new master with a fresh IV;
   - atomic-swaps the new envelope into the SQLite TEXT column inside
     the DO's single-writer scope.
5. Collect per-cell results. When the report has zero failures (or
   operator accepts the reported losses):
   - `wrangler secret put TENANT_MASTER_SEED <new>`
   - `wrangler secret delete TENANT_MASTER_SEED_v2`
   - redeploy.
6. Release the per-tenant write semaphore.

## Operator workflow — engine SystemVerb

The engine SystemVerb is a stateless rotation primitive — it consumes
sealed bytes and emits new sealed bytes. It does NOT touch the storage
backend. Used by the kernel and worker call sites above when their own
rotation paths need to drive the per-cell walk through the engine
boundary (e.g. wasm-bindgen workers, host CLI tools).

```
system(handle, "rotate_tenant_master:<tenant_id>", body) -> envelope
```

Gate: `RegisterMode::Privileged`. An accidentally-exposed `system_impl`
over HTTP/MCP MUST NOT let a remote actor trigger a rotation.

Body shape:

```json
{
  "old": "<64-hex-char master>",
  "new": "<64-hex-char master>",
  "cells": [
    {
      "scope": "...",
      "domain": "...",
      "cell_name": "...",
      "version": 0,
      "sealed_hex": "<lowercase hex of the old sealed envelope>"
    }
  ]
}
```

Success envelope:

```json
{
  "ok": true,
  "tenant_id": "<id>",
  "rotated_count": N,
  "failure_count": M,
  "rotated": [
    {"scope": "...", "domain": "...", "cell_name": "...",
     "version": 0, "sealed_hex": "<hex of the new sealed envelope>"}
  ],
  "failures": [
    {"scope": "...", "domain": "...", "cell_name": "...",
     "version": 0, "kind": "auth" | "truncated"}
  ]
}
```

The `<tenant_id>` suffix is informational (audit trail) — at the
engine layer a SYSTEM call is already scoped to a tenant via its
`handle`. Two tenants on the same handle cannot share a master, so
the suffix is a label for operator logs, not a routing key.

Hex (not base64) for the sealed payload because the engine has no
base64 dep today and the existing `register:<name>` hex-body path
already established the convention. A future base64 upgrade can land
alongside whatever follow-up adds the dep.

## Out of scope (deferred)

- **Zero-downtime rotation**. First version takes the tenant
  read-only for the rotation window via the per-tenant write lock
  described above.
- **Per-cell metadata indicating which master a cell was sealed
  under**. Without it the operator MUST keep both masters loaded
  simultaneously through the rotation. Once promotion lands the old
  master can be zeroized.
- **Automated rotation triggers**. Operator-initiated only, via the
  `SystemVerb::RotateTenantMaster` privileged dispatch.

## Failure modes and recovery

- A `failures` entry with `kind = "auth"` means the old master could
  not open the envelope — most likely a stale row from a previous
  rotation that was never completed, a tampered ciphertext, or a row
  sealed under a third master entirely. Operator decision: retry the
  cell with a different candidate master, zeroize it (delete the row
  and remove the entity from the population), or accept the loss.

- A `failures` entry with `kind = "truncated"` means the storage row
  was structurally malformed (shorter than the 28-byte AEAD overhead).
  Most often a torn DO write or a bad sector — the row is unrecoverable
  regardless of master, so zeroize is the only outcome.

- In all failure cases the row is LEFT UNTOUCHED. The rotation walk
  never destroys data; it produces new sealed bytes only when the old
  master succeeds at opening the original row.
