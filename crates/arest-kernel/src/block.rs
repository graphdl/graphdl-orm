// crates/arest-kernel/src/block.rs
//
// Block device layer for the kernel (#335).
//
// Wraps a `VirtIOBlk` driver behind a spin::Mutex and exposes a small
// sector-oriented API — `read_sector`, `write_sector`, `capacity_sectors`,
// `is_readonly`. The storage layer (`block_storage.rs`, #337) sits on top
// of this primitive; it does not know whether the underlying medium is
// virtio-blk, NVMe, or a file-backed loop image. When virtio-blk is
// absent (no `-device virtio-blk-pci,...` on the QEMU command line) the
// accessors return `Error::NotAvailable` and the rest of the kernel
// continues to boot with in-memory state only.
//
// Contract:
//   * All offsets and counts are in 512-byte `SECTOR_SIZE` units.
//   * `read_sector(n, buf)` requires `buf.len() >= SECTOR_SIZE`; extra
//     bytes are left untouched.
//   * Writes are durable once `flush()` returns; this module exposes
//     `flush()` as the checkpoint fence used by #337.
//   * Concurrent callers serialize through the mutex — virtio-drivers'
//     `VirtIOBlk` is not Send+Sync, so one outstanding request at a time.

use spin::Mutex;
use virtio_drivers::device::blk::SECTOR_SIZE;

use crate::virtio::VirtIOBlkDevice;

pub use virtio_drivers::device::blk::SECTOR_SIZE as BLOCK_SECTOR_SIZE;

/// Errors the block layer can surface. Kept narrow — detailed virtio
/// codes go to the serial log on the way through, so callers only need
/// to distinguish "no device" from "request failed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// No virtio-blk device was found at boot. Boot continues without
    /// persistence; `block_storage::try_rehydrate` treats this as
    /// equivalent to "no checkpoint on disk".
    NotAvailable,
    /// The provided buffer is shorter than the amount of data requested.
    BufferTooSmall,
    /// The driver returned an error — virtio I/O error, request out of
    /// range, device in an unrecoverable state. The specific virtio
    /// code is printed to serial; callers treat this as opaque.
    Io,
    /// A write was attempted against a read-only device (`VIRTIO_BLK_F_RO`
    /// negotiated by the host — typical with `-drive ...,readonly=on`).
    ReadOnly,
}

static DEVICE: Mutex<Option<VirtIOBlkDevice>> = Mutex::new(None);

/// Install the virtio-blk driver. Called once from `kernel_main` after
/// `virtio::try_init_virtio_blk()` has returned a driver. Subsequent
/// calls replace the driver (handy only in test flows; production
/// boots install once).
pub fn install(dev: VirtIOBlkDevice) {
    *DEVICE.lock() = Some(dev);
}

/// True when a virtio-blk device was brought up at boot. The storage
/// layer short-circuits on `false` to avoid surfacing `Error::NotAvailable`
/// from every call.
pub fn available() -> bool {
    DEVICE.lock().is_some()
}

/// Capacity in `SECTOR_SIZE`-byte sectors. Zero when no device is
/// installed.
pub fn capacity_sectors() -> u64 {
    DEVICE.lock().as_ref().map(|d| d.capacity()).unwrap_or(0)
}

/// Is the attached device read-only? False when no device is installed
/// (there's no write surface to distinguish from).
pub fn is_readonly() -> bool {
    DEVICE.lock().as_ref().map(|d| d.readonly()).unwrap_or(false)
}

/// Read a single sector into `buf`. Returns `Error::BufferTooSmall` if
/// `buf.len() < SECTOR_SIZE`. Only the first `SECTOR_SIZE` bytes of
/// `buf` are written.
pub fn read_sector(sector: u64, buf: &mut [u8]) -> Result<(), Error> {
    if buf.len() < SECTOR_SIZE {
        return Err(Error::BufferTooSmall);
    }
    let mut guard = DEVICE.lock();
    let dev = guard.as_mut().ok_or(Error::NotAvailable)?;
    match dev.read_blocks(sector as usize, &mut buf[..SECTOR_SIZE]) {
        Ok(()) => Ok(()),
        Err(e) => {
            crate::println!("  block: read_blocks({sector}) failed: {:?}", e);
            Err(Error::Io)
        }
    }
}

/// Write a single sector from `buf`. Returns `Error::BufferTooSmall`
/// when `buf.len() < SECTOR_SIZE`. Writes are buffered by the device;
/// callers that need durability must call `flush()` afterwards.
pub fn write_sector(sector: u64, buf: &[u8]) -> Result<(), Error> {
    if buf.len() < SECTOR_SIZE {
        return Err(Error::BufferTooSmall);
    }
    let mut guard = DEVICE.lock();
    let dev = guard.as_mut().ok_or(Error::NotAvailable)?;
    if dev.readonly() {
        return Err(Error::ReadOnly);
    }
    match dev.write_blocks(sector as usize, &buf[..SECTOR_SIZE]) {
        Ok(()) => Ok(()),
        Err(e) => {
            crate::println!("  block: write_blocks({sector}) failed: {:?}", e);
            Err(Error::Io)
        }
    }
}

/// Flush pending writes. Called by `block_storage::commit` as the
/// checkpoint fence; after it returns, writes are durable across a
/// power-off.
pub fn flush() -> Result<(), Error> {
    let mut guard = DEVICE.lock();
    let dev = guard.as_mut().ok_or(Error::NotAvailable)?;
    match dev.flush() {
        Ok(()) => Ok(()),
        Err(e) => {
            crate::println!("  block: flush failed: {:?}", e);
            Err(Error::Io)
        }
    }
}
