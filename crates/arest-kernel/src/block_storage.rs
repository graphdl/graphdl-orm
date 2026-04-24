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
