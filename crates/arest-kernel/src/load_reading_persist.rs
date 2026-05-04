// crates/arest-kernel/src/load_reading_persist.rs
//
// DynRdg-T1 (#560) — kernel target persists runtime LoadReading bodies
// and replays them on boot.
//
// FFFFF's #555 (`5b537cc` + `9cfa826`) shipped the runtime LoadReading
// verb in `crates/arest`. The verb threads `(name, body) → (state)
// → Result<(report, new_state), error>` through parse + merge +
// deontic-validation against the live state. End-to-end, calling it
// from the QEMU REPL (`system::load_reading my-app <body>`) extends
// the cell graph live.
//
// Without the persistence layer in this file, that extension is lost
// on the next boot — the kernel re-bakes the static metamodel and the
// dynamically loaded reading evaporates. This module's job is to:
//
//   1. **Persist** every successful LoadReading call into a region of
//      the virtio-blk disk reserved for the loaded-readings ring
//      (sectors 6144..8192 = 1 MiB, sitting in the deliberate slack
//      between the Doom save table and the blob slots).
//   2. **Replay** the persisted record stream on boot, after
//      `system::init()` has built the baked metamodel state, by
//      calling `arest::load_reading::load_reading` against that state
//      once per persisted record (in append order) and committing the
//      merged state via `system::apply`.
//
// Storage layout (append-only):
//
//   Each record:
//     [magic: u32 LE = 0x5244_4741]   // "RDGA" — Reading Append
//     [name_len: u16 LE]
//     [name: name_len bytes UTF-8]
//     [body_len: u32 LE]
//     [body: body_len bytes UTF-8]
//     [version: u32 LE]
//     [tombstone: u8]                 // 0 = load, 1 = unload (tombstone)
//     [crc32: u32 LE]                 // CRC over magic..tombstone
//
//   The region is a flat byte ring sitting on top of `RegionHandle`
//   sector I/O. We page-buffer in 512-byte sectors and walk the
//   ring in record order on replay. A zero magic word terminates the
//   walk (fresh disk = all zeros = no records).
//
// Idempotency: re-loading the same `(name, body)` is a no-op at the
// `load_reading` level (set semantics on Noun / FactType / DerivationRule
// cells — see `crates/arest/src/load_reading.rs::tests::
// re_load_same_body_is_idempotent`). We still append a record so the
// timeline is preserved; the replay handles duplicates harmlessly.
//
// Single-tenant: the kernel runs one tenant. Records are global.
//
// Failure modes:
//   * Corrupt record → log + skip + continue. The next-record offset
//     is computed from the field lengths, so a single corrupt CRC
//     doesn't poison the rest of the ring. (A length-field corruption
//     could; we cap `name_len` and `body_len` at `MAX_NAME_LEN` /
//     `MAX_BODY_BYTES` and treat anything larger as ring termination.)
//   * Replay validation fail (deontic / parse) → log + skip + continue.
//   * No virtio-blk device → silent no-op. Persistence is best-effort
//     and the kernel boots with the baked metamodel only.
//
// What this module DOES NOT do:
//   * Versioning logic (#558) — `version` is stored as `1`
//     unconditionally on the persistence side. The replay surface
//     hands the value back to callers but doesn't make policy
//     decisions on it.
//   * REPL surface (#564) — that's a separate task; this module
//     exposes a Rust API only.
//
// This file lives in the kernel crate, not in `arest`, because it
// reaches `crate::block_storage::RegionHandle` for the actual disk
// I/O — and `arest` is `no_std` + can't depend on the kernel.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::convert::TryFrom;

use arest::ast::Object;

// The runtime LoadReading verb
// (`arest::load_reading_core::load_reading`) became no_std-reachable
// across #588 (`097577ff` — parse_forml2_stage2 no_std) and #589
// (`56c201df` + the engine-side gate-drop commit on the same issue
// — check.rs gate lift + load_reading_core function-body gates
// dropped). Replay calls the verb directly now; the closure-injection
// scaffold is gone (#589 — kernel-side adoption).
//
// Persistence at the byte-stream layer (`persist_record`,
// `decode_all`, `coalesce_live`, `walk_to_end`) is still pure byte
// arithmetic — those helpers run anywhere and are unchanged.

// ── Disk layout constants ──────────────────────────────────────────

/// First sector of the loaded-readings ring. Sits in the slack window
/// between the Doom save table (`SAVE_BASE_SECTOR` = 1024 + 64×65 →
/// 5184 sectors used; ends at 5184) and the blob slot table
/// (`BLOB_BASE_SECTOR` = 8192). The 5184..8192 window was reserved by
/// `block_storage.rs`'s layout note as "slack so a future audit-log
/// primitive can land between Doom and the blob region." This region
/// is the first occupant.
pub const RING_BASE_SECTOR: u64 = 6144;

/// Number of sectors in the ring. 2048 sectors × 512 B = 1 MiB.
/// At ~256 B per typical record (short name + a few short readings),
/// that's space for ~4000 records before the ring exhausts. The MVP
/// behaviour on exhaustion is "stop appending" (records simply fail
/// to land); a future revision can add wrap-around.
pub const RING_SECTOR_COUNT: u64 = 2048;

/// Sector size in bytes. Mirrors `block::BLOCK_SECTOR_SIZE` so we
/// don't import a UEFI-only constant on the host side.
pub const RING_SECTOR_BYTES: usize = 512;

/// Total ring size in bytes. Pre-computed for the in-memory test backend.
pub const RING_BYTES: usize = (RING_SECTOR_COUNT as usize) * RING_SECTOR_BYTES;

/// Magic prefix on every record. "RDGA" — Reading Append. Distinct
/// from the `AREST-K1` checkpoint magic and the `DOOMSAV1` save
/// magic so a misdirected read at the wrong sector is recognized
/// and rejected.
pub const RECORD_MAGIC: u32 = 0x5244_4741;

/// Cap on `name_len`. Anything larger is treated as ring termination
/// (the field is corrupt or we've walked past the last record into
/// zero-fill / garbage). 256 B is well above the largest reasonable
/// reading name.
pub const MAX_NAME_LEN: usize = 256;

/// Cap on `body_len`. 64 KiB is the inline-vs-region threshold used
/// elsewhere in `block_storage`; readings larger than this should
/// be loaded once at bake time, not appended to the ring.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// Tombstone marker — record represents an unload of a previously-
/// loaded reading. Pre-reserved for #556 (UnloadReading). Replay
/// treats a tombstone'd name by skipping any earlier record carrying
/// that name; the cumulative "live readings" set is the set of
/// load records whose name is not later tombstone'd.
pub const TOMBSTONE_LIVE: u8 = 0;
pub const TOMBSTONE_DEAD: u8 = 1;

// ── Record encoding ────────────────────────────────────────────────

/// One persisted record as it lives in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedReading {
    pub name: String,
    pub body: String,
    pub version: u32,
    pub tombstone: u8,
}

impl PersistedReading {
    /// Encoded length on disk: magic(4) + name_len(2) + name + body_len(4)
    /// + body + version(4) + tombstone(1) + crc(4).
    pub fn encoded_len(&self) -> usize {
        4 + 2 + self.name.len() + 4 + self.body.len() + 4 + 1 + 4
    }

    /// Append the canonical byte encoding to `out`. Returns
    /// `Some(bytes_appended)` (== `encoded_len`) on success, `None`
    /// when `name.len() > MAX_NAME_LEN` or `body.len() > MAX_BODY_BYTES`
    /// — the bounds the wire format can address. Single source of
    /// truth: `persist_record` short-circuits before calling, but
    /// direct callers (boot replay, future verb handlers, fuzz
    /// fixtures) get the same guard without panicking (#708).
    pub fn encode_to(&self, out: &mut Vec<u8>) -> Option<usize> {
        if self.name.len() > MAX_NAME_LEN || self.body.len() > MAX_BODY_BYTES {
            return None;
        }
        let start = out.len();
        out.extend_from_slice(&RECORD_MAGIC.to_le_bytes());
        // Bounds above guarantee the casts fit — name_len ≤ 256 ≤ u16::MAX,
        // body_len ≤ 64 KiB ≤ u32::MAX. `as` is safe inside this gate.
        let nl = self.name.len() as u16;
        out.extend_from_slice(&nl.to_le_bytes());
        out.extend_from_slice(self.name.as_bytes());
        let bl = self.body.len() as u32;
        out.extend_from_slice(&bl.to_le_bytes());
        out.extend_from_slice(self.body.as_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.push(self.tombstone);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_le_bytes());
        Some(out.len() - start)
    }
}

/// Outcome of a single decode attempt against a byte cursor.
#[derive(Debug)]
enum DecodeStep {
    /// Successfully decoded a record; cursor advanced by `consumed`.
    Record { record: PersistedReading, consumed: usize },
    /// Hit zero magic / corrupt length / past end — the walk should
    /// stop here. Not an error per se: a fresh ring is all zeros and
    /// returns `End` on the very first read.
    End,
    /// CRC mismatch on a record whose lengths were within bounds.
    /// Caller logs and skips by `consumed` bytes.
    Skip { consumed: usize },
}

fn decode_at(buf: &[u8], offset: usize) -> DecodeStep {
    // Need at least magic + name_len to even peek.
    let header_min = 4 + 2;
    if buf.len() < offset + header_min {
        return DecodeStep::End;
    }
    let magic = u32::from_le_bytes([
        buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3],
    ]);
    if magic != RECORD_MAGIC {
        return DecodeStep::End;
    }
    let name_len = u16::from_le_bytes([buf[offset + 4], buf[offset + 5]]) as usize;
    if name_len > MAX_NAME_LEN {
        return DecodeStep::End;
    }
    let body_len_off = offset + 6 + name_len;
    if buf.len() < body_len_off + 4 {
        return DecodeStep::End;
    }
    let body_len = u32::from_le_bytes([
        buf[body_len_off], buf[body_len_off + 1],
        buf[body_len_off + 2], buf[body_len_off + 3],
    ]) as usize;
    if body_len > MAX_BODY_BYTES {
        return DecodeStep::End;
    }
    // version(4) + tombstone(1) + crc(4) = 9 trailing bytes.
    let body_off = body_len_off + 4;
    let version_off = body_off + body_len;
    let tombstone_off = version_off + 4;
    let crc_off = tombstone_off + 1;
    if buf.len() < crc_off + 4 {
        return DecodeStep::End;
    }
    let total_len = crc_off + 4 - offset;

    let name_bytes = &buf[offset + 6..offset + 6 + name_len];
    let name = match core::str::from_utf8(name_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return DecodeStep::Skip { consumed: total_len },
    };
    let body_bytes = &buf[body_off..body_off + body_len];
    let body = match core::str::from_utf8(body_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return DecodeStep::Skip { consumed: total_len },
    };
    let version = u32::from_le_bytes([
        buf[version_off], buf[version_off + 1],
        buf[version_off + 2], buf[version_off + 3],
    ]);
    let tombstone = buf[tombstone_off];
    let stored_crc = u32::from_le_bytes([
        buf[crc_off], buf[crc_off + 1], buf[crc_off + 2], buf[crc_off + 3],
    ]);
    let computed_crc = crc32(&buf[offset..crc_off]);
    if stored_crc != computed_crc {
        return DecodeStep::Skip { consumed: total_len };
    }

    DecodeStep::Record {
        record: PersistedReading { name, body, version, tombstone },
        consumed: total_len,
    }
}

/// Walk a byte buffer (the in-memory ring snapshot) and decode every
/// record. Returns the records in append order; corrupt records are
/// silently skipped. The walk terminates at the first zero-magic or
/// out-of-range length field — i.e. the first byte past the last
/// record.
pub fn decode_all(buf: &[u8]) -> Vec<PersistedReading> {
    let mut records = Vec::new();
    let mut offset = 0;
    loop {
        match decode_at(buf, offset) {
            DecodeStep::Record { record, consumed } => {
                records.push(record);
                offset += consumed;
            }
            DecodeStep::Skip { consumed } => {
                offset += consumed;
            }
            DecodeStep::End => break,
        }
    }
    records
}

/// Coalesce a record stream into the live-reading set. Later records
/// overwrite earlier ones with the same `name`; tombstone'd names
/// are removed from the live set. Returns records in original append
/// order, filtered down to the live set.
///
/// Uses `BTreeMap` (not `HashMap`) so the kernel build doesn't need
/// to pull `hashbrown` for this single map — `BTreeMap` is in
/// `alloc::collections` and works under no_std without an extra dep.
pub fn coalesce_live(records: &[PersistedReading]) -> Vec<PersistedReading> {
    use alloc::collections::BTreeMap;

    // First pass: index the LATEST record per name (preserves the
    // most recent body/version/tombstone for each name).
    let mut latest: BTreeMap<&str, &PersistedReading> = BTreeMap::new();
    for rec in records {
        latest.insert(rec.name.as_str(), rec);
    }
    // Second pass: walk the original order, emitting only the entry
    // whose pointer matches the latest map AND that is not a
    // tombstone. The order-preserving emit keeps replay deterministic.
    let mut out = Vec::new();
    for rec in records {
        if let Some(&lr) = latest.get(rec.name.as_str()) {
            if core::ptr::eq(lr as *const _, rec as *const _)
                && rec.tombstone == TOMBSTONE_LIVE
            {
                out.push(rec.clone());
            }
        }
    }
    out
}

// ── Storage backend trait ──────────────────────────────────────────
//
// Tests run on the host (no virtio-blk), so the storage interface is
// abstracted behind a tiny `RingBackend` trait. The production impl
// (`VirtioBlkRing`) lives behind the same UEFI gate that
// `block_storage::RegionHandle` does; the test impl
// (`InMemoryRing`) is always available.

/// Backend that knows how to read the entire ring and how to append
/// fresh bytes after the last record. Both methods are infallible
/// from the caller's perspective — implementation errors (no device,
/// I/O failure) surface as either an empty read or a no-op append,
/// both of which the persistence layer treats as "best-effort
/// degraded" rather than a kernel panic.
pub trait RingBackend {
    /// Read the entire ring contents into a fresh `Vec<u8>`. Length
    /// is `RING_BYTES` on a real device; tests may shrink for speed.
    fn read_all(&self) -> Vec<u8>;
    /// Append `bytes` immediately after the last record. The backend
    /// is responsible for finding that offset (typically by walking
    /// `read_all` itself; the in-memory test impl just tracks an
    /// internal cursor). Returns the offset at which the append
    /// landed, or `None` on failure (ring full, no device, etc.).
    fn append(&mut self, bytes: &[u8]) -> Option<usize>;
}

/// In-memory ring used by tests. Production callers reach for
/// `VirtioBlkRing` instead.
#[derive(Debug, Clone)]
pub struct InMemoryRing {
    pub bytes: Vec<u8>,
    pub cursor: usize,
}

impl Default for InMemoryRing {
    fn default() -> Self {
        Self {
            bytes: vec![0u8; RING_BYTES],
            cursor: 0,
        }
    }
}

impl InMemoryRing {
    /// Build a ring pre-seeded with `initial` bytes at offset 0;
    /// cursor advances past it. Useful for tests that want to
    /// simulate a non-empty ring on boot.
    pub fn with_initial(initial: &[u8]) -> Self {
        let mut bytes = vec![0u8; RING_BYTES];
        let n = core::cmp::min(initial.len(), RING_BYTES);
        bytes[..n].copy_from_slice(&initial[..n]);
        Self { bytes, cursor: n }
    }

    /// Replace the underlying byte buffer wholesale. Test helper for
    /// "what if the disk had this content on boot" scenarios.
    pub fn set_bytes(&mut self, bytes: Vec<u8>) {
        let len = bytes.len();
        let mut padded = bytes;
        if len < RING_BYTES {
            padded.resize(RING_BYTES, 0);
        } else {
            padded.truncate(RING_BYTES);
        }
        self.bytes = padded;
        // Re-seek cursor past the last record on the new buffer.
        self.cursor = walk_to_end(&self.bytes);
    }
}

/// Walk a buffer past its last well-formed record (or first corrupt
/// stretch) and return the resulting offset. Used by both
/// `InMemoryRing::set_bytes` and the production append path so a
/// remount can pick up where the previous boot left off.
pub fn walk_to_end(buf: &[u8]) -> usize {
    let mut offset = 0;
    loop {
        match decode_at(buf, offset) {
            DecodeStep::Record { consumed, .. } => offset += consumed,
            DecodeStep::Skip { consumed } => offset += consumed,
            DecodeStep::End => return offset,
        }
    }
}

impl RingBackend for InMemoryRing {
    fn read_all(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn append(&mut self, bytes: &[u8]) -> Option<usize> {
        if self.cursor + bytes.len() > self.bytes.len() {
            return None;
        }
        let landed_at = self.cursor;
        self.bytes[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.cursor += bytes.len();
        Some(landed_at)
    }
}

// ── Production backend (UEFI x86_64 only) ──────────────────────────
//
// The virtio-blk-backed ring carves the [RING_BASE_SECTOR,
// RING_BASE_SECTOR + RING_SECTOR_COUNT) range out of the persistence
// disk via `block_storage::reserve_region`. On every `append` it
// rounds the encoded record up to the next sector boundary,
// overwrites that span, and updates the in-memory cursor. On
// `read_all` it pulls the entire region in one block of sector
// reads.
//
// The append path is deliberately sector-granular (one record per
// sector group, padded with zeros) so a torn write never bleeds
// into the *next* record's header. The trade-off is wasted bytes
// per record (up to 511 B); fine for the MVP, since record count
// is bounded by reading load frequency in practice (a handful per
// boot).

#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub struct VirtioBlkRing {
    region: crate::block_storage::RegionHandle,
    cursor: usize,
}

#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
impl VirtioBlkRing {
    /// Reserve the loaded-readings region against the persistence
    /// disk. Returns `None` when no virtio-blk device is attached or
    /// the reservation fails (disk too small).
    pub fn open() -> Option<Self> {
        let region = crate::block_storage::reserve_region(
            RING_BASE_SECTOR,
            RING_SECTOR_COUNT,
        )
        .ok()?;
        // Read the whole region to seek the cursor past the last
        // record (so we don't overwrite previous boot's records).
        let mut buf = vec![0u8; RING_BYTES];
        // RegionHandle::read works on whole sectors; RING_BYTES is
        // sector-aligned by construction so this is safe.
        if region.read(0, &mut buf).is_err() {
            // Treat I/O failure as a fresh ring; the kernel surfaces
            // a banner line elsewhere and we don't want to panic.
            return Some(Self { region, cursor: 0 });
        }
        let cursor = walk_to_end(&buf);
        Some(Self { region, cursor })
    }
}

#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
impl RingBackend for VirtioBlkRing {
    fn read_all(&self) -> Vec<u8> {
        let mut buf = vec![0u8; RING_BYTES];
        let _ = self.region.read(0, &mut buf);
        buf
    }

    fn append(&mut self, bytes: &[u8]) -> Option<usize> {
        if self.cursor + bytes.len() > RING_BYTES {
            return None;
        }
        // Round to sector boundary on both ends so the write is
        // sector-aligned (RegionHandle::write demands it).
        let start_sector = self.cursor / RING_SECTOR_BYTES;
        let start_off_in_sector = self.cursor % RING_SECTOR_BYTES;
        let end = self.cursor + bytes.len();
        let end_sector = (end + RING_SECTOR_BYTES - 1) / RING_SECTOR_BYTES;
        let n_sectors = end_sector - start_sector;
        let mut sector_buf = vec![0u8; n_sectors * RING_SECTOR_BYTES];
        // Read existing sectors so we preserve the pre-cursor portion
        // of the first sector when our record starts mid-sector.
        if self.region
            .read(start_sector as u64, &mut sector_buf)
            .is_err()
        {
            return None;
        }
        sector_buf[start_off_in_sector..start_off_in_sector + bytes.len()]
            .copy_from_slice(bytes);
        if self.region
            .write(start_sector as u64, &sector_buf)
            .is_err()
        {
            return None;
        }
        let _ = self.region.flush();
        let landed_at = self.cursor;
        self.cursor = end;
        Some(landed_at)
    }
}

// ── Public API ─────────────────────────────────────────────────────

/// Append a `(name, body, version)` record into `backend`. Returns
/// `true` when the append landed, `false` when the ring is full or
/// the backend refused. `tombstone` is `TOMBSTONE_LIVE` for a normal
/// load; `TOMBSTONE_DEAD` for an unload (#556 will surface a
/// tombstone-emitting verb).
pub fn persist_record(
    backend: &mut dyn RingBackend,
    name: &str,
    body: &str,
    version: u32,
    tombstone: u8,
) -> bool {
    if name.is_empty() || body.is_empty() {
        return false;
    }
    if name.len() > MAX_NAME_LEN || body.len() > MAX_BODY_BYTES {
        return false;
    }
    let rec = PersistedReading {
        name: name.to_string(),
        body: body.to_string(),
        version,
        tombstone,
    };
    let mut bytes = Vec::with_capacity(rec.encoded_len());
    if rec.encode_to(&mut bytes).is_none() {
        return false;
    }
    backend.append(&bytes).is_some()
}

/// Convenience wrapper for callers that always want a live load
/// record at version 1 (current MVP behaviour — versioning lands in
/// #558). Same return contract as `persist_record`.
pub fn persist_loaded_reading(
    backend: &mut dyn RingBackend,
    name: &str,
    body: &str,
    version: u32,
) -> bool {
    persist_record(backend, name, body, version, TOMBSTONE_LIVE)
}

/// Replay every live record from `backend` by invoking
/// `arest::load_reading_core::load_reading` against `state` for each
/// `(name, body, _version)` triple. On success, `state` is updated
/// to the post-load state; on any `LoadError`, the record is skipped
/// and replay continues with the next one so a single bad record
/// can't poison the boot.
///
/// (#589) The earlier closure-injection signature exposed an
/// `F: FnMut(&Object, &str, &str, u32) -> Result<Object, ()>`
/// parameter because `load_reading` was gated behind
/// `not(feature = "no_std")` and the kernel build couldn't reach
/// it. With the gate dropped (parse + check both no_std-reachable
/// across #588 + #589), the closure scaffold is no longer needed —
/// callers stop wiring `apply` themselves and the function
/// dispatches directly through the verb.
///
/// `version` from the persisted record is intentionally not threaded
/// into `load_reading` — the verb allocates its own version stamp
/// (`next_version()`) per the #558 monotonicity contract. Callers
/// that need to seed the counter from the persisted floor should do
/// that via `arest::load_reading_core::seed_version_counter` ahead
/// of replay.
///
/// Policy is `LoadReadingPolicy::AllowAll` — replay is host-trusted
/// (the records came from a previous successful load on this same
/// kernel, so `Deny` would be incorrect).
///
/// Returns the count of records that successfully merged (i.e. the
/// verb returned `Ok`).
pub fn replay_loaded_readings(
    backend: &dyn RingBackend,
    state: &mut Object,
) -> usize {
    use arest::load_reading_core::{load_reading, LoadReadingPolicy};

    let buf = backend.read_all();
    let records = decode_all(&buf);
    let live = coalesce_live(&records);
    let mut applied = 0usize;
    for rec in live {
        match load_reading(state, &rec.name, &rec.body, LoadReadingPolicy::AllowAll) {
            Ok(outcome) => {
                *state = outcome.new_state;
                applied += 1;
            }
            Err(_) => {
                // Best-effort: skip this record and continue. A
                // future revision can route the error to the
                // kernel's diagnostic buffer; for the MVP we
                // silently drop so a transient bad record can't
                // wedge boot.
                continue;
            }
        }
    }
    applied
}

/// Walk the ring and return every live record, without applying any
/// of them. Useful for kernel diagnostics ("how many readings would
/// be replayed if the verb were wired up") and for the eventual #564
/// REPL surface that consumes the records itself.
pub fn live_records(backend: &dyn RingBackend) -> Vec<PersistedReading> {
    let buf = backend.read_all();
    let records = decode_all(&buf);
    coalesce_live(&records)
}

/// Production-side replay entry point used by the UEFI boot sequence.
/// Opens the virtio-blk-backed ring, walks every persisted record,
/// and replays each one through
/// `arest::load_reading_core::load_reading` against the live
/// `crate::system` state. (#589 — replay actually applies records
/// now; pre-#589 behaviour was "report count only" because the verb
/// was gated on std.)
///
/// Returns `Ok(applied_count)` on success — the number of records
/// the verb merged in, which may be lower than the live record
/// count when individual bodies fail the load-time gate (#559)
/// against the merged state. Returns `Err(reason)` when the kernel
/// can't even open the ring (no device, etc.).
///
/// Failure-mode contract (unchanged from #560): "corrupt region →
/// log + continue with bake-time only. Replay validation fail →
/// log + skip + continue."
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub fn replay_from_disk() -> Result<usize, &'static str> {
    let backend = VirtioBlkRing::open()
        .ok_or("loaded-readings ring: no virtio-blk device or reservation refused")?;

    // Snapshot the live state, replay against the snapshot, commit
    // the merged state in one `system::apply`. We can't hold the
    // SYSTEM read lock across `replay_loaded_readings` because the
    // verb itself walks the state (and the system module's apply
    // path takes the write lock); cloning the snapshot via
    // `with_state` is the clean separation.
    let mut state = crate::system::with_state(|s| s.clone())
        .ok_or("system::init() not called before replay_from_disk")?;
    let applied = replay_loaded_readings(&backend, &mut state);
    if applied > 0 {
        crate::system::apply(state)
            .map_err(|_| "system::apply failed during replay commit")?;
    }
    Ok(applied)
}

// ── CRC-32/IEEE ────────────────────────────────────────────────────
//
// Mirrors `block_storage::crc32` — same polynomial, same shape.
// Kept local so this module doesn't reach into `block_storage`'s
// private surface.

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{self, fact_from_pairs};

    /// Minimal seed state mirroring `arest::load_reading::tests::seed_state`.
    fn seed_state() -> Object {
        let nouns = ast::Object::seq(vec![ast::fact_from_pairs(&[
            ("name", "Order"),
            ("objectType", "entity"),
        ])]);
        ast::store("Noun", nouns, &Object::phi())
    }

    /// #708 / Audit P2: `encode_to` must return None instead of
    /// panicking when name or body exceed the on-disk bounds. Direct
    /// callers (boot replay, future verb handlers, fuzz fixtures)
    /// hit this path without going through `persist_record`'s gate.
    #[test]
    fn encode_to_returns_none_on_oversized_name() {
        let huge_name = "x".repeat(MAX_NAME_LEN + 1);
        let rec = PersistedReading {
            name: huge_name,
            body: "Order(.id) is an entity type.\n".to_string(),
            version: 1,
            tombstone: TOMBSTONE_LIVE,
        };
        let mut bytes = Vec::new();
        assert_eq!(rec.encode_to(&mut bytes), None);
        assert!(bytes.is_empty(), "rejected encode must not partially write");
    }

    #[test]
    fn encode_to_returns_none_on_oversized_body() {
        let huge_body = "x".repeat(MAX_BODY_BYTES + 1);
        let rec = PersistedReading {
            name: "ok".to_string(),
            body: huge_body,
            version: 1,
            tombstone: TOMBSTONE_LIVE,
        };
        let mut bytes = Vec::new();
        assert_eq!(rec.encode_to(&mut bytes), None);
        assert!(bytes.is_empty(), "rejected encode must not partially write");
    }

    /// Encode + decode round-trip on a single record. Bytes go in,
    /// the same record comes out.
    #[test]
    fn encode_decode_round_trip() {
        let rec = PersistedReading {
            name: "catalog".to_string(),
            body: "Product(.SKU) is an entity type.\n".to_string(),
            version: 1,
            tombstone: TOMBSTONE_LIVE,
        };
        let mut bytes = Vec::new();
        let n = rec.encode_to(&mut bytes).expect("fixture within bounds");
        assert_eq!(n, rec.encoded_len(), "encoded len matches reported");

        let recs = decode_all(&bytes);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0], rec);
    }

    /// Fresh (zero-filled) ring decodes to an empty record list.
    #[test]
    fn fresh_ring_yields_no_records() {
        let backend = InMemoryRing::default();
        let buf = backend.read_all();
        let recs = decode_all(&buf);
        assert!(recs.is_empty(), "fresh ring should decode to zero records");
    }

    /// Append + read-back through the in-memory backend.
    #[test]
    fn append_then_read_back() {
        let mut backend = InMemoryRing::default();
        assert!(persist_loaded_reading(
            &mut backend,
            "catalog",
            "Product(.SKU) is an entity type.\n",
            1,
        ));
        let buf = backend.read_all();
        let recs = decode_all(&buf);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].name, "catalog");
        assert_eq!(recs[0].version, 1);
        assert_eq!(recs[0].tombstone, TOMBSTONE_LIVE);
    }

    /// Multiple appends preserve order on read-back.
    #[test]
    fn multi_record_preserves_order() {
        let mut backend = InMemoryRing::default();
        persist_loaded_reading(&mut backend, "a", "Alpha(.Name) is an entity type.\n", 1);
        persist_loaded_reading(&mut backend, "b", "Beta(.Name) is an entity type.\n", 1);
        persist_loaded_reading(&mut backend, "c", "Gamma(.Name) is an entity type.\n", 1);

        let recs = decode_all(&backend.read_all());
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].name, "a");
        assert_eq!(recs[1].name, "b");
        assert_eq!(recs[2].name, "c");
    }

    /// Corrupting the CRC of one record should not poison the rest of
    /// the ring — replay walks past the bad record and continues.
    #[test]
    fn corrupt_crc_skipped_gracefully() {
        let mut backend = InMemoryRing::default();
        persist_loaded_reading(&mut backend, "good_a", "Alpha(.Name) is an entity type.\n", 1);
        persist_loaded_reading(&mut backend, "bad_b", "Beta(.Name) is an entity type.\n", 1);
        persist_loaded_reading(&mut backend, "good_c", "Gamma(.Name) is an entity type.\n", 1);

        // Corrupt the CRC of the middle record. We need to find its
        // offset; the easiest path is to walk the buffer ourselves.
        let mut buf = backend.read_all();
        let recs_before = decode_all(&buf);
        assert_eq!(recs_before.len(), 3);
        // Compute offset of the second record by encoded_len of the first.
        let first_len = recs_before[0].encoded_len();
        // Corrupt the CRC bytes (last 4) of the second record.
        let second_len = recs_before[1].encoded_len();
        let crc_pos = first_len + second_len - 4;
        for i in 0..4 {
            buf[crc_pos + i] ^= 0xFF;
        }
        backend.set_bytes(buf);

        // Replay: the corrupt record is skipped; the third record is
        // preserved. Note: the in-memory backend's `set_bytes` walks
        // to the end of the (now-corrupted) sequence, so the cursor
        // sits past the third record.
        let recs_after = decode_all(&backend.read_all());
        assert_eq!(recs_after.len(), 2, "corrupt record skipped, others preserved");
        assert_eq!(recs_after[0].name, "good_a");
        assert_eq!(recs_after[1].name, "good_c");
    }

    /// Length-field corruption (name_len > MAX_NAME_LEN) terminates
    /// the walk — defends against a torn write on the length prefix.
    #[test]
    fn oversize_name_len_terminates_walk() {
        let mut backend = InMemoryRing::default();
        persist_loaded_reading(&mut backend, "good", "Alpha(.Name) is an entity type.\n", 1);
        // Patch name_len to a huge value at offset 4..6 (right after magic).
        let mut buf = backend.read_all();
        buf[4] = 0xFF;
        buf[5] = 0xFF;
        backend.set_bytes(buf);

        let recs = decode_all(&backend.read_all());
        assert!(recs.is_empty(), "oversize name_len terminates walk");
    }

    /// Replay against the seed state via the direct call to
    /// `arest::load_reading_core::load_reading`. (#589 — closure
    /// scaffold dropped; verb is no_std-reachable now.) Each
    /// persisted single-line entity declaration parses + merges
    /// through the actual verb pipeline.
    #[test]
    fn replay_against_seed_state_via_load_reading() {
        let mut backend = InMemoryRing::default();
        persist_loaded_reading(
            &mut backend,
            "catalog",
            "Product(.SKU) is an entity type.\n",
            1,
        );
        persist_loaded_reading(
            &mut backend,
            "tax",
            "TaxRate(.Code) is an entity type.\n",
            1,
        );

        let mut state = seed_state();
        let applied = replay_loaded_readings(&backend, &mut state);
        assert_eq!(applied, 2);

        // The merged state has Order (from seed) + Product + TaxRate.
        let nouns = ast::fetch_or_phi("Noun", &state);
        let names: Vec<&str> = nouns
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"), "seed Order preserved");
        assert!(names.contains(&"Product"), "Product loaded");
        assert!(names.contains(&"TaxRate"), "TaxRate loaded");
    }

    /// Records carrying bodies that fail the load-time gate
    /// (`LoadError`) are skipped — replay walks the entire record
    /// stream and surfaces only the successful loads in `applied`.
    /// (#589) Replaces the old closure-error test; the closure is
    /// gone, so we use a body the verb actually rejects (a
    /// reserved-keyword noun declaration → `LoadError::ParseError`).
    #[test]
    fn replay_skips_load_failures() {
        let mut backend = InMemoryRing::default();
        // First record parses fine — should land.
        persist_loaded_reading(
            &mut backend,
            "good",
            "Alpha(.Name) is an entity type.\n",
            1,
        );
        // Second record uses `each` as a noun declaration — #309
        // reserved-keyword check rejects with `LoadError::ParseError`.
        persist_loaded_reading(
            &mut backend,
            "bad",
            "each(.X) is an entity type.\n",
            1,
        );

        let mut state = seed_state();
        let applied = replay_loaded_readings(&backend, &mut state);
        assert_eq!(
            applied, 1,
            "only the good record applies; bad one is skipped"
        );

        // Alpha noun landed; the rejected body left no trace.
        let nouns = ast::fetch_or_phi("Noun", &state);
        let names: Vec<&str> = nouns
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Alpha"), "Alpha loaded from good record");
        assert!(!names.contains(&"each"), "rejected body did not land");
    }

    /// `live_records` returns the coalesced record set without any
    /// state mutation — this is the surface the eventual #564 REPL
    /// hook will read to drive its own apply loop.
    #[test]
    fn live_records_returns_coalesced_set() {
        let mut backend = InMemoryRing::default();
        persist_loaded_reading(&mut backend, "a", "Alpha(.Name) is an entity type.\n", 1);
        persist_loaded_reading(&mut backend, "b", "Beta(.Name) is an entity type.\n", 1);
        // Tombstone "a" — should drop from live set.
        let dead_a = PersistedReading {
            name: "a".to_string(),
            body: "(unloaded)".to_string(),
            version: 1,
            tombstone: TOMBSTONE_DEAD,
        };
        let mut bytes = Vec::new();
        dead_a.encode_to(&mut bytes).expect("fixture within bounds");
        backend.append(&bytes);

        let live = live_records(&backend);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "b");
    }

    /// A tombstone'd record removes the prior load from the live set
    /// — the replay skips the now-dead reading entirely.
    #[test]
    fn tombstone_removes_prior_load() {
        let mut backend = InMemoryRing::default();
        persist_record(
            &mut backend,
            "catalog",
            "Product(.SKU) is an entity type.\n",
            1,
            TOMBSTONE_LIVE,
        );
        // Append a tombstone with empty body intentionally rejected
        // by `persist_record` (body must be non-empty), so we hand-
        // roll the record here. The tombstone body is the record
        // marker itself; its content is informational only.
        let dead = PersistedReading {
            name: "catalog".to_string(),
            body: "(unloaded)".to_string(),
            version: 1,
            tombstone: TOMBSTONE_DEAD,
        };
        let mut bytes = Vec::new();
        dead.encode_to(&mut bytes).expect("fixture within bounds");
        backend.append(&bytes);

        let recs = decode_all(&backend.read_all());
        assert_eq!(recs.len(), 2, "both records persist");

        let live = coalesce_live(&recs);
        assert!(live.is_empty(), "tombstone removes 'catalog' from live set");
    }

    /// Re-load with a different body keeps only the latest body in
    /// the live set (later overwrites earlier under same name).
    #[test]
    fn later_record_overrides_earlier_for_same_name() {
        let mut backend = InMemoryRing::default();
        persist_loaded_reading(
            &mut backend,
            "catalog",
            "Product(.SKU) is an entity type.\n",
            1,
        );
        persist_loaded_reading(
            &mut backend,
            "catalog",
            "Catalog(.Name) is an entity type.\n",
            2,
        );

        let recs = decode_all(&backend.read_all());
        assert_eq!(recs.len(), 2);

        let live = coalesce_live(&recs);
        assert_eq!(live.len(), 1, "only latest record per name in live set");
        assert_eq!(live[0].body, "Catalog(.Name) is an entity type.\n");
        assert_eq!(live[0].version, 2);
    }

    /// Empty `name` or `body` is rejected by `persist_record` —
    /// ring stays empty.
    #[test]
    fn empty_name_or_body_rejected() {
        let mut backend = InMemoryRing::default();
        assert!(!persist_loaded_reading(&mut backend, "", "body", 1));
        assert!(!persist_loaded_reading(&mut backend, "name", "", 1));
        assert_eq!(decode_all(&backend.read_all()).len(), 0);
    }

    /// `walk_to_end` on a ring with two records sits at the
    /// post-second-record offset.
    #[test]
    fn walk_to_end_lands_past_last_record() {
        let mut backend = InMemoryRing::default();
        persist_loaded_reading(
            &mut backend,
            "a",
            "Alpha(.Name) is an entity type.\n",
            1,
        );
        persist_loaded_reading(
            &mut backend,
            "b",
            "Beta(.Name) is an entity type.\n",
            1,
        );

        let buf = backend.read_all();
        let recs = decode_all(&buf);
        let expected = recs.iter().map(|r| r.encoded_len()).sum::<usize>();
        assert_eq!(walk_to_end(&buf), expected);
    }

    /// Suppress unused-import warning for `fact_from_pairs` on builds
    /// where the seed_state helper is the only consumer (it is, but
    /// the linter sometimes flags top-level use blocks).
    #[allow(dead_code)]
    fn _silence_unused() {
        let _ = fact_from_pairs(&[("k", "v")]);
    }
}
