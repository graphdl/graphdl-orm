// crates/arest-kernel/src/block_storage.rs
//
// Storage-4 boot-time mount + checkpoint/restore (#337).
//
// Layers on top of `block` (#335) to give the kernel a persistence
// contract shaped like the std-side `StorageBackend` (#334) — without
// pulling in alloc-hungry trait objects or the `StorageBackend` types
// themselves, which are gated on std.
//
// Layout:
//
//   Sector 0 — Header {
//     magic:       [u8; 8]   // "AREST-K1"
//     version:     u16 LE    // 1
//     flags:       u16 LE    // bit 0: clean_shutdown
//     data_len:    u32 LE    // total state bytes following sector 0
//     data_crc32:  u32 LE    // CRC-32/IEEE of the state bytes
//     boot_count:  u64 LE    // +1 on every successful boot
//     reserved:    [u8; 480] // zero
//   }
//   Sectors 1..N — State bytes (padded with 0x00 to the next sector).
//
// On boot, `mount()` reads sector 0, validates the header, and:
//   * If magic mismatches → "no checkpoint on disk"; `rehydrate()` is
//     a no-op and the kernel proceeds with its baked state. The first
//     `checkpoint()` call seeds the header.
//   * If magic matches and CRC passes → returns `Some(bytes)`; caller
//     (system::init) can `freeze::thaw` those bytes and replace its
//     initial state.
//   * If CRC fails → log the mismatch and report `Err(Corrupted)`; we
//     deliberately do not silently overwrite, so a bad sector forces
//     operator action.
//
// `checkpoint(bytes)` writes the new state + updated header back and
// bumps `boot_count`. `flush` returns after the virtio request queue
// reports done, so durability holds across a power-off.
//
// The kernel's `system::init` is a `spin::Once<Object>` — immutable
// after boot — so there's no per-commit path to hook today. #337 ships
// the primitive; `system::init_with_persistence` in this commit calls
// `mount()` once, and on the single boot cycle also exercises a
// round-trip `checkpoint(state_marker) → read_state_marker` to prove
// the pipeline is live. Once the kernel grows mutable state (Sec-6
// userspace / #333, or a kernel REPL with persistence), the same
// checkpoint entrypoint is what the commit path calls.

use alloc::vec::Vec;

use crate::block::{self, BLOCK_SECTOR_SIZE};
use arest::cell_aead::{self, CellAddress, TenantMasterKey};

/// Eight-byte magic prefix. Keyed to AREST + kernel (K) + schema
/// version 1. Distinct from the freeze-byte `AREST\x01` marker so a
/// consumer that peeled the wrong layer fails fast.
pub const MAGIC: &[u8; 8] = b"AREST-K1";

/// Sector 0 header length. Fits inside one sector by construction.
pub const HEADER_LEN: usize = 512;

/// Bytes reserved at the start of sector 0 for the fixed header
/// fields (magic..reserved). The rest of sector 0 is zero-padded.
const HEADER_FIXED_LEN: usize = 8 + 2 + 2 + 4 + 4 + 8;

/// Bit 0 of the `flags` field — set by the clean-shutdown hook, cleared
/// on every boot. A non-clean shutdown flag in the loaded header means
/// the kernel crashed between the last checkpoint and a final
/// `mark_clean_shutdown` call; callers can use it to trigger audit-log
/// replay.
pub const FLAG_CLEAN_SHUTDOWN: u16 = 1 << 0;

/// Mount result — what `mount()` surfaced on the current boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountStatus {
    /// No virtio-blk device is attached. Boot proceeds with baked
    /// state only; no checkpoints are ever written.
    NoDevice,
    /// Disk present but magic doesn't match — first boot against a
    /// fresh disk. The kernel writes an initial header on the first
    /// `checkpoint()` call.
    FreshDisk,
    /// Disk had a valid header — `mount()` has loaded it. `last_state`
    /// returns the recovered state bytes on success.
    Rehydrated,
    /// Header magic matched but CRC failed. The kernel refuses to
    /// silently overwrite; surfaces `MountStatus::Corrupted` so the
    /// operator can inspect.
    Corrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The block layer was unavailable or returned an I/O failure.
    Io,
    /// The in-memory state blob exceeds the on-disk slab (capacity
    /// minus the header sector). Increase the disk size at QEMU-
    /// invocation time.
    StateTooLarge,
    /// Checkpoint called before a successful mount. Protects against
    /// writing a header to an uninitialized mount. Also surfaced from
    /// `reserve_region` when no virtio-blk device is attached — callers
    /// get a hard error rather than a silent-drop handle.
    NotMounted,
    /// A `RegionHandle` operation addressed a sector outside the
    /// reserved range, or was passed a buffer whose length is not a
    /// whole multiple of `BLOCK_SECTOR_SIZE`. Callers that want
    /// variable-length payloads layer framing on top of the region.
    OutOfRange,
    /// AEAD seal / open of the on-disk state blob failed (#659).
    /// `Truncated` here means the sealed envelope on disk was
    /// shorter than the AEAD overhead; `Auth` means the tag /
    /// AAD didn't match — most often a stale tenant master key
    /// after a salt rotation. Callers should refuse to rehydrate
    /// rather than silently boot from the baked metamodel.
    Aead(cell_aead::AeadError),
}

// ── Mount state ────────────────────────────────────────────────────

use spin::Mutex;

struct Mounted {
    status: MountStatus,
    /// Header at last read. `boot_count` is already bumped on boot —
    /// callers see the value that was stored during the *previous*
    /// session.
    header: Header,
    /// Rehydrated bytes. `None` on `FreshDisk` / `NoDevice` / `Corrupted`.
    state: Option<Vec<u8>>,
}

static MOUNT: Mutex<Option<Mounted>> = Mutex::new(None);

#[derive(Debug, Clone, Copy)]
struct Header {
    magic: [u8; 8],
    version: u16,
    flags: u16,
    data_len: u32,
    data_crc32: u32,
    boot_count: u64,
}

impl Header {
    fn fresh() -> Self {
        Self {
            magic: *MAGIC,
            version: 1,
            flags: 0,
            data_len: 0,
            data_crc32: 0,
            boot_count: 0,
        }
    }

    fn encode(self, buf: &mut [u8; BLOCK_SECTOR_SIZE]) {
        buf[..HEADER_FIXED_LEN].fill(0);
        buf[HEADER_FIXED_LEN..].fill(0);
        let mut i = 0;
        buf[i..i + 8].copy_from_slice(&self.magic); i += 8;
        buf[i..i + 2].copy_from_slice(&self.version.to_le_bytes()); i += 2;
        buf[i..i + 2].copy_from_slice(&self.flags.to_le_bytes()); i += 2;
        buf[i..i + 4].copy_from_slice(&self.data_len.to_le_bytes()); i += 4;
        buf[i..i + 4].copy_from_slice(&self.data_crc32.to_le_bytes()); i += 4;
        buf[i..i + 8].copy_from_slice(&self.boot_count.to_le_bytes());
    }

    fn decode(buf: &[u8; BLOCK_SECTOR_SIZE]) -> Self {
        let mut i = 0;
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&buf[i..i + 8]); i += 8;
        let version = u16::from_le_bytes([buf[i], buf[i + 1]]); i += 2;
        let flags = u16::from_le_bytes([buf[i], buf[i + 1]]); i += 2;
        let data_len = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]); i += 4;
        let data_crc32 = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]); i += 4;
        let boot_count = u64::from_le_bytes([
            buf[i], buf[i + 1], buf[i + 2], buf[i + 3],
            buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7],
        ]);
        Self { magic, version, flags, data_len, data_crc32, boot_count }
    }
}

/// Mount the persistence disk. Called once from `kernel_main` after
/// `block::install`. Idempotent on repeated calls — subsequent calls
/// return the cached status without re-reading sector 0.
pub fn mount() -> MountStatus {
    {
        let guard = MOUNT.lock();
        if let Some(m) = guard.as_ref() {
            return m.status;
        }
    }

    if !block::available() {
        *MOUNT.lock() = Some(Mounted {
            status: MountStatus::NoDevice,
            header: Header::fresh(),
            state: None,
        });
        return MountStatus::NoDevice;
    }

    let mut sector0 = [0u8; BLOCK_SECTOR_SIZE];
    if block::read_sector(0, &mut sector0).is_err() {
        *MOUNT.lock() = Some(Mounted {
            status: MountStatus::NoDevice,
            header: Header::fresh(),
            state: None,
        });
        return MountStatus::NoDevice;
    }

    let header = Header::decode(&sector0);
    if header.magic != *MAGIC {
        *MOUNT.lock() = Some(Mounted {
            status: MountStatus::FreshDisk,
            header: Header::fresh(),
            state: None,
        });
        return MountStatus::FreshDisk;
    }

    let data_len = header.data_len as usize;
    let sectors_needed = (data_len + BLOCK_SECTOR_SIZE - 1) / BLOCK_SECTOR_SIZE;
    let mut state: Vec<u8> = Vec::with_capacity(sectors_needed * BLOCK_SECTOR_SIZE);
    let mut sector_buf = [0u8; BLOCK_SECTOR_SIZE];
    for s in 0..sectors_needed {
        if block::read_sector(1 + s as u64, &mut sector_buf).is_err() {
            *MOUNT.lock() = Some(Mounted {
                status: MountStatus::NoDevice,
                header,
                state: None,
            });
            return MountStatus::NoDevice;
        }
        state.extend_from_slice(&sector_buf);
    }
    state.truncate(data_len);

    let crc = crc32(&state);
    let status = if crc == header.data_crc32 {
        MountStatus::Rehydrated
    } else {
        crate::println!(
            "  blk:    checkpoint CRC mismatch (header={:#010x}, computed={:#010x})",
            header.data_crc32, crc,
        );
        MountStatus::Corrupted
    };

    let stored_state = match status {
        MountStatus::Rehydrated => Some(state),
        _ => None,
    };
    *MOUNT.lock() = Some(Mounted {
        status,
        header,
        state: stored_state,
    });
    status
}

/// Borrow the rehydrated state bytes from the last `mount()`. Returns
/// `None` when there's nothing to rehydrate (fresh disk, missing
/// device, or corrupted checkpoint).
pub fn last_state() -> Option<Vec<u8>> {
    MOUNT.lock().as_ref().and_then(|m| m.state.clone())
}

/// Boot counter observed at the start of the current boot — i.e. how
/// many times the kernel booted *before* this one. Zero on first boot
/// against a fresh disk; non-zero boots after the first `checkpoint()`
/// call runs.
pub fn last_boot_count() -> u64 {
    MOUNT.lock().as_ref().map(|m| m.header.boot_count).unwrap_or(0)
}

/// Status from the last `mount()`. Kept for banner reporting.
pub fn status() -> MountStatus {
    MOUNT.lock().as_ref().map(|m| m.status).unwrap_or(MountStatus::NoDevice)
}

/// Persist `state` as the current checkpoint. Writes sectors 1..N,
/// then updates the header with new length, CRC, and an incremented
/// boot counter. Returns `StateTooLarge` when the state exceeds the
/// disk's data area.
pub fn checkpoint(state: &[u8]) -> Result<(), Error> {
    if !block::available() {
        return Err(Error::NotMounted);
    }
    let capacity_sectors = block::capacity_sectors();
    if capacity_sectors < 2 {
        return Err(Error::StateTooLarge);
    }
    let data_sectors = capacity_sectors - 1;
    let data_capacity = data_sectors * BLOCK_SECTOR_SIZE as u64;
    if state.len() as u64 > data_capacity {
        return Err(Error::StateTooLarge);
    }

    // Write state into sectors 1..N. Pad the final sector with 0x00.
    let mut sector = [0u8; BLOCK_SECTOR_SIZE];
    let mut offset = 0;
    let mut s = 0u64;
    while offset < state.len() {
        let chunk = core::cmp::min(BLOCK_SECTOR_SIZE, state.len() - offset);
        sector[..chunk].copy_from_slice(&state[offset..offset + chunk]);
        if chunk < BLOCK_SECTOR_SIZE {
            sector[chunk..].fill(0);
        }
        block::write_sector(1 + s, &sector).map_err(|_| Error::Io)?;
        offset += chunk;
        s += 1;
    }

    // Rewrite header last so a torn write leaves the previous header
    // intact rather than claiming data that wasn't fully landed.
    let prev_boot_count = last_boot_count();
    let header = Header {
        magic: *MAGIC,
        version: 1,
        flags: 0,
        data_len: state.len() as u32,
        data_crc32: crc32(state),
        boot_count: prev_boot_count.saturating_add(1),
    };
    let mut sector0 = [0u8; BLOCK_SECTOR_SIZE];
    header.encode(&mut sector0);
    block::write_sector(0, &sector0).map_err(|_| Error::Io)?;
    block::flush().map_err(|_| Error::Io)?;

    // Refresh the cache so subsequent `last_state` / `last_boot_count`
    // reflect the just-written header.
    *MOUNT.lock() = Some(Mounted {
        status: MountStatus::Rehydrated,
        header,
        state: Some(state.to_vec()),
    });
    Ok(())
}

/// Mark the current session as having shut down cleanly. Sets bit 0 of
/// the header `flags`. Called from the QEMU-exit path when the kernel
/// teardown reaches the clean-shutdown gate. #337's audit-log replay
/// inspects this flag on the next boot.
#[allow(dead_code)]
pub fn mark_clean_shutdown() -> Result<(), Error> {
    if !block::available() {
        return Err(Error::NotMounted);
    }
    let mut sector0 = [0u8; BLOCK_SECTOR_SIZE];
    block::read_sector(0, &mut sector0).map_err(|_| Error::Io)?;
    let mut header = Header::decode(&sector0);
    if header.magic != *MAGIC {
        return Err(Error::NotMounted);
    }
    header.flags |= FLAG_CLEAN_SHUTDOWN;
    header.encode(&mut sector0);
    block::write_sector(0, &sector0).map_err(|_| Error::Io)?;
    block::flush().map_err(|_| Error::Io)?;
    Ok(())
}

// ── Sealed checkpoint (#659) ───────────────────────────────────────
//
// `checkpoint(bytes)` above writes plaintext sector by sector. Every
// serialization boundary outside the engine's in-memory operating
// set must seal each cell before it leaves, so the kernel
// persistence path also wraps the whole state blob in an AEAD
// envelope keyed against a per-tenant master.
//
// The freeze layer already seals each cell individually
// (`arest::freeze::freeze_sealed`), so this layer's outer-envelope
// AEAD looks like double-encryption — and it is, deliberately:
//
//   * Inner (freeze_sealed): each cell sealed against
//     CellAddress(scope, domain, cell_name, cell_version). Defends
//     a leaked sector or a torn-write region against per-cell
//     surface area.
//   * Outer (checkpoint_sealed): the entire freeze blob sealed as a
//     single anonymous cell at CellAddress("kernel",
//     "persistence", "checkpoint", boot_count). Defends the layout
//     metadata (number of cells, cell-name strings, sealed-cell
//     lengths) — all of which freeze_sealed leaves in plaintext as
//     routing keys.
//
// Cost is one extra HKDF derivation + one ChaCha20-Poly1305 pass
// over the whole blob per checkpoint (~tens of KB/s on a software
// implementation), and 28 bytes of envelope overhead. The on-disk
// CRC + magic stay over the *sealed* bytes — wire-level corruption
// detection happens before AEAD open, the AEAD path defends only
// against a bit-perfect read of unauthorised data.
//
// Callers that want plaintext checkpoints (boot-image bake, FPGA
// ROM image, in-process snapshot debugging) keep using `checkpoint`
// / `last_state`; encryption-required paths (the production boot
// loop, every host that mounts persistent storage) should reach for
// `checkpoint_sealed` / `last_state_sealed_open`.

/// AAD/salt domain string for the kernel persistence checkpoint —
/// scopes the outer AEAD envelope to this exact use site so a
/// future "cluster replication" or "cold-start migration" wrapper
/// over the same master can't open a checkpoint envelope by accident.
pub const CHECKPOINT_SCOPE: &str = "kernel";

/// Sub-domain for the persistence layer specifically. Pairs with
/// `CHECKPOINT_SCOPE` to form the outer-envelope cell address.
pub const CHECKPOINT_DOMAIN: &str = "persistence";

/// Cell name for the single anonymous checkpoint cell. Constant
/// because there's only ever one of these on disk per slot — multi-
/// slot checkpoint follow-ups (#666 and friends) would extend this
/// with a slot id.
pub const CHECKPOINT_CELL: &str = "checkpoint";

/// Persist `state` as the current checkpoint, sealing the whole
/// blob against `master` first. The version field of the address
/// is the previous boot count (so each checkpoint binds to the boot
/// it was minted under) — a kernel that rolls back through a
/// reboot will read the right version off the header before
/// re-deriving the per-cell key.
///
/// Same `Error::StateTooLarge` budget as `checkpoint` minus the
/// AEAD overhead. `Error::Aead` is reserved for the sister
/// `last_state_sealed_open` path; the seal direction never fails
/// for finite inputs (the underlying RustCrypto encrypt is
/// infallible).
pub fn checkpoint_sealed(state: &[u8], master: &TenantMasterKey) -> Result<(), Error> {
    let prev_boot = last_boot_count();
    let address = CellAddress::new(
        CHECKPOINT_SCOPE,
        CHECKPOINT_DOMAIN,
        CHECKPOINT_CELL,
        prev_boot,
    );
    let sealed = cell_aead::cell_seal(master, &address, state);
    checkpoint(&sealed)
}

/// Recover and AEAD-open the last sealed checkpoint. Returns `Ok(None)`
/// when there's nothing to rehydrate (fresh disk, no device,
/// corrupted header — same shape as `last_state`); returns
/// `Err(Error::Aead(_))` when the on-disk envelope is structurally
/// fine but the AEAD tag / AAD doesn't match the master.
///
/// The address `version` is the boot counter recorded in the
/// header, NOT the live `last_boot_count` (which is one greater
/// after a successful boot). Without this, a kernel that rebooted
/// between the seal and the open would derive a different per-cell
/// key and fail Auth on its own checkpoint.
pub fn last_state_sealed_open(master: &TenantMasterKey) -> Result<Option<Vec<u8>>, Error> {
    let sealed = match last_state() {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    // The sealed-envelope's address binds to the *previous* boot
    // count — not the current one. The mount step already loaded
    // the on-disk header (which carries the `boot_count` value
    // committed by the previous session); reuse that here so the
    // rehydration AAD matches what the seal step wrote.
    let prev_boot = MOUNT
        .lock()
        .as_ref()
        .map(|m| m.header.boot_count)
        .unwrap_or(0);
    let address = CellAddress::new(
        CHECKPOINT_SCOPE,
        CHECKPOINT_DOMAIN,
        CHECKPOINT_CELL,
        prev_boot,
    );
    cell_aead::cell_open(master, &address, &sealed)
        .map(Some)
        .map_err(Error::Aead)
}

// ── Tenant master rotation (#662 — kernel side) ────────────────────
//
// First-version rotation takes the kernel checkpoint slot read-only
// for the duration of the call. The kernel's mount slot is guarded
// by `MOUNT: Mutex<Option<Mounted>>` (single-writer by construction)
// and `checkpoint_sealed` / `last_state_sealed_open` already
// serialise on the same lock, so the rotation walk inherits the
// serialisation without an extra primitive — we just hold the
// outer rotate call long enough to do read-old → write-new without
// an interleaving checkpoint.
//
// Operator workflow (kernel):
//
//   1. Persist the new 32-byte master in the freeze-blob "pending"
//      slot alongside the existing "active" slot. (Targets that
//      derive the master from boot entropy + salt store the new
//      salt in "pending" instead of the bytes themselves.)
//   2. Call `rotate_checkpoint_master(old, new)`:
//        - reads the on-disk sealed envelope under `old`;
//        - re-seals the recovered plaintext under `new`;
//        - writes the new envelope back atomically (single
//          `checkpoint()` call — same path the rest of the kernel
//          uses, so the durability contract is unchanged).
//   3. Promote "pending" → "active" in the freeze-blob.
//   4. Wipe the old master from the boot path.
//
// Only ONE sealed cell on disk: the kernel persists a single
// outer-envelope checkpoint at `("kernel", "persistence",
// "checkpoint", boot_count)`. A multi-checkpoint follow-up (#666 et
// al.) would extend this to the per-slot checkpoint set.

/// Rotate the on-disk checkpoint envelope from `old` to `new`. Reads
/// the sealed bytes off the mount, opens with the old master,
/// re-seals with the new master, writes the new envelope back.
///
/// Returns `Ok(())` on a clean rotation; `Err(Error::NotMounted)` if
/// no checkpoint has ever been written; `Err(Error::Aead(_))` if the
/// old master cannot open the on-disk envelope (in which case the
/// disk is left untouched — operator must intervene).
///
/// The rotation holds the mount lock (`MOUNT`) for the read-side
/// only. The write-back hands off through `checkpoint()` which
/// re-acquires it; in between, a concurrent `checkpoint_sealed`
/// call from the kernel's own tick would be locked out by the
/// Mutex, so the read-old / write-new sequence is atomic against
/// other writers. First-version assumption, documented above.
pub fn rotate_checkpoint_master(
    old: &TenantMasterKey,
    new: &TenantMasterKey,
) -> Result<(), Error> {
    // Read side: pull the sealed bytes + the boot_count they were
    // sealed at.
    let (sealed, prev_boot) = {
        let guard = MOUNT.lock();
        match guard.as_ref() {
            Some(m) => match &m.state {
                Some(bytes) => (bytes.clone(), m.header.boot_count),
                None => return Err(Error::NotMounted),
            },
            None => return Err(Error::NotMounted),
        }
    };
    let address = CellAddress::new(
        CHECKPOINT_SCOPE,
        CHECKPOINT_DOMAIN,
        CHECKPOINT_CELL,
        prev_boot,
    );
    // Re-seal under `new` using the rotation primitive — single-cell
    // atomic, fresh nonce, same address (so AAD bindings stay
    // consistent). A `Truncated` envelope on disk is impossible at
    // this point (CRC + magic already passed at mount time), but
    // surface it cleanly anyway via `Error::Aead`.
    let new_sealed = cell_aead::rotate_cell(old, new, &address, &sealed)
        .map_err(Error::Aead)?;
    // Write back through the existing checkpoint path — re-uses the
    // CRC + header machinery so the on-disk format is identical.
    checkpoint(&new_sealed)?;
    // Refresh the in-memory mount cache so a subsequent
    // `last_state_sealed_open(new)` reads the just-written envelope.
    {
        let mut guard = MOUNT.lock();
        if let Some(m) = guard.as_mut() {
            m.state = Some(new_sealed);
        }
    }
    Ok(())
}

// ── Reserved sub-regions ───────────────────────────────────────────
//
// `reserve_region` carves a contiguous sector range out of the
// virtio-blk disk and hands back a `RegionHandle` with its own
// sector-oriented read/write/flush verbs. It exists so persistence
// clients other than the #337 kernel checkpoint (Doom saves #375,
// future config storage, log rotation, etc.) can live on the same
// disk without stepping on the checkpoint header/body.
//
// Footprint of the existing checkpoint (callers must avoid this
// window when choosing a `base_sector`):
//
//   * Sector 0          — checkpoint header (`AREST-K1` magic + CRC).
//   * Sectors 1..=N     — checkpoint state body, where `N` is bounded
//                         by `block::capacity_sectors() - 1`.
//
// The checkpoint body grows up to the full disk minus one sector
// (see `checkpoint()` above), so a shared disk layout MUST either
// cap the checkpoint size (the kernel's state blob is KB-scale today,
// comfortably inside a few hundred sectors) or pick a disk size
// generous enough that high-sector regions don't collide. As a rule
// of thumb Doom and other callers should base their regions well
// above the checkpoint's practical upper bound — e.g. start at
// sector 1024 on the ≥8 MiB disks the harness configures, which
// leaves 512 KiB for checkpoint growth.
//
// There is no on-disk header or CRC at the `RegionHandle` layer.
// Clients that want framing/versioning/integrity layer it themselves
// (Doom's save-file format already carries its own header).

/// Carve a reserved sub-range of the virtio-blk disk. Returns an
/// owning handle that routes all subsequent I/O through the block
/// layer with an offset applied. Collisions with the checkpoint
/// region (sector 0 + body) are the caller's responsibility — see the
/// module-level footprint note above.
///
/// Fails with:
///   * `Error::NotMounted` — no virtio-blk device installed, so any
///     write would be silently dropped. We surface the absence up
///     front rather than handing back a stub handle.
///   * `Error::OutOfRange` — the requested `[base, base+count)` range
///     exceeds the disk capacity, or `sector_count == 0`.
///
/// Zero capacity is rejected because the block layer is wedged in
/// that state (a zero-capacity disk fails the first `read_sector`
/// anyway); callers get the cleaner error shape from here.
#[allow(dead_code)]
pub fn reserve_region(base_sector: u64, sector_count: u64) -> Result<RegionHandle, Error> {
    if !block::available() {
        return Err(Error::NotMounted);
    }
    if sector_count == 0 {
        return Err(Error::OutOfRange);
    }
    let end = base_sector
        .checked_add(sector_count)
        .ok_or(Error::OutOfRange)?;
    let capacity = block::capacity_sectors();
    if end > capacity {
        return Err(Error::OutOfRange);
    }
    Ok(RegionHandle { base_sector, sector_count })
}

/// Owning handle over a reserved sector range of the virtio-blk
/// disk. Instances are small (two `u64`s) and freely copyable — the
/// block device itself lives behind `block::DEVICE` globally, so a
/// `RegionHandle` doesn't borrow it and has no lifetime. Dropping a
/// handle does nothing; two handles over overlapping ranges both see
/// the same on-disk bytes, which is by design (Doom's save-slot code
/// happily drops a handle mid-boot and reconstructs a fresh one from
/// (base, count) on the next save).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct RegionHandle {
    base_sector: u64,
    sector_count: u64,
}

#[allow(dead_code)]
impl RegionHandle {
    /// Base sector on the underlying device. Primarily useful for
    /// logging + tests; production callers should prefer `read` /
    /// `write`, which apply the offset for them.
    pub fn base_sector(&self) -> u64 {
        self.base_sector
    }

    /// Size of the reserved range in 512-byte sectors.
    pub fn sector_count(&self) -> u64 {
        self.sector_count
    }

    /// Size of the reserved range in bytes. `sector_count *
    /// BLOCK_SECTOR_SIZE`, pre-computed for convenience.
    pub fn len_bytes(&self) -> u64 {
        self.sector_count * BLOCK_SECTOR_SIZE as u64
    }

    /// Read `buf.len() / BLOCK_SECTOR_SIZE` sectors starting at
    /// `offset_sector` within the region into `buf`. `buf.len()` must
    /// be a whole multiple of `BLOCK_SECTOR_SIZE` and the read must
    /// stay inside the reserved range.
    pub fn read(&self, offset_sector: u64, buf: &mut [u8]) -> Result<(), Error> {
        let n_sectors = self.check_range(offset_sector, buf.len())?;
        for s in 0..n_sectors {
            let start = (s as usize) * BLOCK_SECTOR_SIZE;
            let end = start + BLOCK_SECTOR_SIZE;
            block::read_sector(
                self.base_sector + offset_sector + s,
                &mut buf[start..end],
            )
            .map_err(|_| Error::Io)?;
        }
        Ok(())
    }

    /// Write `data.len() / BLOCK_SECTOR_SIZE` sectors starting at
    /// `offset_sector` within the region from `data`. Same length /
    /// range rules as `read`. Callers that need durability must call
    /// `flush` afterwards.
    pub fn write(&self, offset_sector: u64, data: &[u8]) -> Result<(), Error> {
        let n_sectors = self.check_range(offset_sector, data.len())?;
        for s in 0..n_sectors {
            let start = (s as usize) * BLOCK_SECTOR_SIZE;
            let end = start + BLOCK_SECTOR_SIZE;
            block::write_sector(
                self.base_sector + offset_sector + s,
                &data[start..end],
            )
            .map_err(|_| Error::Io)?;
        }
        Ok(())
    }

    /// Flush pending writes to durable storage. Forwards straight to
    /// `block::flush` — the virtio-blk driver has no concept of
    /// per-region flush, so this is a whole-device fence. Cheap but
    /// not free; batch writes and call once at the end.
    pub fn flush(&self) -> Result<(), Error> {
        block::flush().map_err(|_| Error::Io)
    }

    /// Shared range + multiple-of-sector check for read/write. On
    /// success returns the number of sectors the operation will
    /// touch.
    fn check_range(&self, offset_sector: u64, byte_len: usize) -> Result<u64, Error> {
        if byte_len == 0 {
            // Zero-length I/O is a no-op; allow it so callers can
            // pass empty slices for "flush only" semantics without
            // special-casing.
            return Ok(0);
        }
        if byte_len % BLOCK_SECTOR_SIZE != 0 {
            return Err(Error::OutOfRange);
        }
        let n_sectors = (byte_len / BLOCK_SECTOR_SIZE) as u64;
        let end = offset_sector
            .checked_add(n_sectors)
            .ok_or(Error::OutOfRange)?;
        if end > self.sector_count {
            return Err(Error::OutOfRange);
        }
        Ok(n_sectors)
    }
}

// ── Blob slot allocator (#401) ─────────────────────────────────────
//
// Layered on top of `reserve_region` to back `File.content_ref`
// region storage (readings/filesystem.md). The encoding spec lives
// in that reading; the kernel side ships only the allocator and the
// fixed slot table.
//
// Layout:
//
//   * Slots are fixed-size, fixed-position. Slot `i` covers sectors
//     [BLOB_BASE_SECTOR + i * BLOB_SLOT_SECTORS,
//      BLOB_BASE_SECTOR + (i + 1) * BLOB_SLOT_SECTORS).
//   * BLOB_BASE_SECTOR = 8192. Sits well above the Doom save table
//     (1024..=5184 — see `doom::save_slot_region`) and the #337
//     checkpoint footprint (sector 0 + a few hundred sectors of
//     body). The window 5185..8191 is intentionally left as slack
//     so a future audit-log primitive can land between Doom and
//     the blob region without re-shuffling.
//   * BLOB_SLOT_BYTES = 256 KiB per slot. Bigger than the 64 KiB
//     inline-vs-region threshold by a factor of 4 so the typical
//     "just over inline" file (a small image, a 100 KiB CSV) fits
//     in one slot with headroom.
//   * BLOB_SLOT_COUNT = 256. 256 × 256 KiB = 64 MiB of region-
//     backed storage. End sector = 8192 + 256 × 512 = 139_264 →
//     requires a virtio-blk disk ≥ ~80 MiB. Smaller disks fail
//     allocation and the consumer is expected to fall back to
//     inline-only storage (which holds ≤ 64 KiB per file).
//
// Free-list state is a fixed-size `[bool; 256]` behind a
// `spin::Mutex` — small, contention-free in practice (the kernel
// is currently single-threaded outside IRQ context), and sized to
// match `BLOB_SLOT_COUNT` exactly so a slot-id always indexes
// safely.
//
// KNOWN LIMITATION: the free-list is in-memory only. After a
// reboot it appears empty, even if the rehydrated `File` table
// references slots that were live before the crash. The follow-up
// (file-ops layer) is responsible for walking the rehydrated File
// rows and re-marking their slots used during `mount` — that's why
// `alloc_region` is exposed as a separate primitive rather than
// being driven from inside this module.

/// First sector of the blob slot table. Picked well clear of the
/// Doom save region (`doom::SAVE_BASE_SECTOR` = 1024 with 64-slot
/// stride 65 → 5184 sectors used) and the #337 checkpoint
/// footprint, with slack between for a future audit-log primitive.
pub const BLOB_BASE_SECTOR: u64 = 8192;

/// Bytes per blob slot. 256 KiB — chosen as 4× the 64 KiB inline-
/// vs-region threshold so a "just over inline" file fits with
/// headroom, and small enough that the full table (256 × 256 KiB
/// = 64 MiB) fits inside the 80 MiB disk size that the harness
/// can be asked to provision for filesystem-enabled boots.
pub const BLOB_SLOT_BYTES: u64 = 256 * 1024;

/// Per-slot stride in 512-byte sectors. Pre-computed so the
/// arithmetic is obvious at the call site.
pub const BLOB_SLOT_SECTORS: u64 = BLOB_SLOT_BYTES / BLOCK_SECTOR_SIZE as u64;

/// Number of slots in the table. 256 × 256 KiB = 64 MiB region-
/// backed blob storage ceiling. A consumer that needs more either
/// shares slots (multi-blob-per-slot framing) or waits for the
/// bitmap-allocator follow-up.
pub const BLOB_SLOT_COUNT: usize = 256;

/// Inline-vs-region cut-over threshold the file-ops encoder uses
/// when deciding which `ContentRef` shape to emit. Documented here
/// (rather than in the encoder) because the on-disk allocator
/// needs to know it to size slots sensibly. 64 KiB = one DO write
/// payload, comfortable Vec resize bound, small enough not to
/// bloat in-memory cells that hold inline blobs.
pub const BLOB_INLINE_MAX_BYTES: u64 = 64 * 1024;

/// In-memory free-list. `false` = free, `true` = allocated. Sized
/// to exactly `BLOB_SLOT_COUNT` so a `slot_id < BLOB_SLOT_COUNT`
/// check is sufficient before indexing.
static BLOB_SLOTS: Mutex<[bool; BLOB_SLOT_COUNT]> = Mutex::new([false; BLOB_SLOT_COUNT]);

/// Allocate a region large enough to hold `byte_len` bytes of blob
/// payload. The returned `RegionHandle` covers exactly one slot —
/// `BLOB_SLOT_BYTES` of contiguous on-disk storage — regardless of
/// the requested length, so a caller that writes a 100 KiB blob
/// into a slot-sized region simply leaves the trailing bytes
/// undefined (this layer carries no length framing; the file-ops
/// `ContentRef::Region { byte_len, .. }` field tracks the live
/// length).
///
/// Fails with:
///   * `Error::StateTooLarge` — `byte_len > BLOB_SLOT_BYTES`. A
///     follow-up may grow the slot or chain multiple slots; for
///     now the consumer must reject the file.
///   * `Error::OutOfRange` — every slot is allocated, or the
///     virtio-blk disk is too small for the slot table (the
///     underlying `reserve_region` call refuses). Callers that
///     get this error should either fall back to inline storage
///     (when the blob fits the inline threshold) or surface the
///     out-of-space condition.
///   * `Error::NotMounted` — propagated from `reserve_region`
///     when no virtio-blk device is installed.
#[allow(dead_code)]
pub fn alloc_region(byte_len: u64) -> Result<RegionHandle, Error> {
    if byte_len > BLOB_SLOT_BYTES {
        return Err(Error::StateTooLarge);
    }
    let slot_id = {
        let mut slots = BLOB_SLOTS.lock();
        let free = slots.iter().position(|&used| !used);
        match free {
            Some(i) => {
                slots[i] = true;
                i
            }
            None => return Err(Error::OutOfRange),
        }
    };
    let base = BLOB_BASE_SECTOR + (slot_id as u64) * BLOB_SLOT_SECTORS;
    match reserve_region(base, BLOB_SLOT_SECTORS) {
        Ok(handle) => Ok(handle),
        Err(e) => {
            // Roll back the in-memory mark so the slot doesn't leak
            // when the underlying disk refuses the range. Without
            // this, a too-small disk would burn through the slot
            // table on repeated alloc attempts.
            BLOB_SLOTS.lock()[slot_id] = false;
            Err(e)
        }
    }
}

/// Return a previously-allocated region to the free-list. The
/// `handle` must have come from `alloc_region` — handles minted
/// directly via `reserve_region` outside the slot table will fail
/// with `Error::OutOfRange`.
///
/// Fails with:
///   * `Error::OutOfRange` — `handle.base_sector` is not aligned
///     to a slot boundary, or its slot id is out of range, or the
///     handle's `sector_count` doesn't match `BLOB_SLOT_SECTORS`.
///   * (No `Error::NotMounted` — freeing is purely an in-memory
///     bookkeeping op and works even when the disk has gone away
///     mid-session.)
///
/// Double-free is detected: freeing an already-free slot returns
/// `Error::OutOfRange`. The kernel does not panic, so a buggy
/// consumer can't take down the system through repeat-frees.
#[allow(dead_code)]
pub fn free_region(handle: RegionHandle) -> Result<(), Error> {
    let base = handle.base_sector();
    if handle.sector_count() != BLOB_SLOT_SECTORS {
        return Err(Error::OutOfRange);
    }
    if base < BLOB_BASE_SECTOR {
        return Err(Error::OutOfRange);
    }
    let offset = base - BLOB_BASE_SECTOR;
    if offset % BLOB_SLOT_SECTORS != 0 {
        return Err(Error::OutOfRange);
    }
    let slot_id = (offset / BLOB_SLOT_SECTORS) as usize;
    if slot_id >= BLOB_SLOT_COUNT {
        return Err(Error::OutOfRange);
    }
    let mut slots = BLOB_SLOTS.lock();
    if !slots[slot_id] {
        // Double-free — the slot was already free. Surface as
        // OutOfRange so the consumer notices, but don't poison
        // the slot.
        return Err(Error::OutOfRange);
    }
    slots[slot_id] = false;
    Ok(())
}

/// Number of currently-allocated blob slots. Useful for boot-time
/// banner reporting and for the future free-list-rehydrate hook
/// to confirm post-mount state matches the rehydrated File table.
#[allow(dead_code)]
pub fn blob_slots_in_use() -> usize {
    BLOB_SLOTS.lock().iter().filter(|&&used| used).count()
}

// ── CRC-32/IEEE ────────────────────────────────────────────────────
//
// Textbook table-free implementation. Fine for the MVP checkpoint
// path — the kernel's state-blob size is KB-scale for the foreseeable
// future and the CRC runs once per commit. If that becomes a hot
// path, swap in a table-driven version.

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

// ── Boot-time smoke (end-to-end proof) ─────────────────────────────

/// Exercise write + read-back against the disk so the first boot
/// surfaces any wiring issue in the virtio-blk stack. Runs once from
/// `kernel_main` after `mount()`. Returns true on success, false on
/// any step failure — both branches print a serial line the E2E
/// harness keys on.
pub fn smoke_round_trip() -> bool {
    if !block::available() {
        return false;
    }
    let probe = b"arest-kernel persistence round-trip marker v1";
    if checkpoint(probe).is_err() {
        crate::println!("  blk:    round-trip checkpoint failed");
        return false;
    }
    // `checkpoint` already refreshed the in-memory cache. Re-read
    // sector 0 + state sectors to prove the values on the wire match.
    let mut sector0 = [0u8; BLOCK_SECTOR_SIZE];
    if block::read_sector(0, &mut sector0).is_err() {
        crate::println!("  blk:    round-trip header read failed");
        return false;
    }
    let header = Header::decode(&sector0);
    if header.magic != *MAGIC || header.data_len as usize != probe.len() {
        crate::println!(
            "  blk:    round-trip header mismatch (magic ok={}, len={})",
            header.magic == *MAGIC, header.data_len,
        );
        return false;
    }
    let mut read_back: Vec<u8> = Vec::with_capacity(probe.len());
    let sectors_needed = (probe.len() + BLOCK_SECTOR_SIZE - 1) / BLOCK_SECTOR_SIZE;
    let mut sb = [0u8; BLOCK_SECTOR_SIZE];
    for s in 0..sectors_needed {
        if block::read_sector(1 + s as u64, &mut sb).is_err() {
            crate::println!("  blk:    round-trip data read failed at sector {}", 1 + s);
            return false;
        }
        read_back.extend_from_slice(&sb);
    }
    read_back.truncate(probe.len());
    if read_back != probe {
        crate::println!("  blk:    round-trip payload mismatch");
        return false;
    }
    if crc32(&read_back) != header.data_crc32 {
        crate::println!("  blk:    round-trip CRC mismatch");
        return false;
    }
    true
}
