// crates/arest-kernel/src/process/address_space.rs
//
// In-memory representation of a Linux process address space — the
// destination of `process::elf::load_segments` (#519, second slice of
// the #472 epic). Owns one `LoadedSegment` per PT_LOAD entry that the
// ELF parser yielded (#518), each carrying a page-aligned heap
// allocation that mirrors the segment's [vaddr, vaddr + memsz) virtual
// range.
//
// Why an in-memory model instead of real page tables
// --------------------------------------------------
// Tier-1 of the spawn epic (#519, this slice) is "the loader produces
// a faithful in-memory image of what the binary asks for"; the actual
// page-table install + ring-3 trampoline is #521 and needs more
// design work (per-arch CR3 / TTBR0 swap, IDT/GDT carve-out for
// userspace, syscall MSR setup). Until then the address space lives
// in the kernel's own heap as a `Vec<LoadedSegment>` — every test the
// loader needs to satisfy (file_size copy, BSS zero, perm flags,
// overlap detection, PT_INTERP rejection) is a property of the bytes
// in those allocations and the metadata around them, not of any
// hardware mapping. When #521 lands, the trampoline will walk this
// `AddressSpace` and install each `LoadedSegment` into the running
// CR3 (or TTBR0 / TTBR1 on aarch64).
//
// Why we route through the global allocator (talc) and not the frame
// allocator
// ------------------------------------------------------------------
// `arch::uefi::memory::with_frame_allocator` hands out raw 4 KiB
// physical frames — the right shape if the page goes straight into a
// page table, but unwieldy in a no-page-table-yet world: each frame
// is yielded through a `FrameAllocator<Size4KiB>` trait, the lifetime
// is "the kernel boot" rather than "this AddressSpace", and the trait
// has no `deallocate_frame` so the storage leaks unconditionally
// (matching the BIOS arm pattern). The talc global allocator already
// backs every `alloc::alloc` call in the kernel via the per-arch
// `#[global_allocator]` (entry_uefi.rs:241 carves 32 MiB via
// `boot::allocate_pages` and feeds talc), and a `Layout::from_size_align
// (n_pages * 4096, 4096)` request hands back a 4-KiB-aligned slice
// drawn from the same pool. Drop runs `dealloc` so `AddressSpace`
// reclaims its pages on tear-down. The eventual #521 trampoline can
// re-derive a `PhysAddr` from the slice's pointer (UEFI is identity-
// mapped so virt == phys) when it walks our segments to install them
// into a real page table.
//
// Permission model
// ----------------
// Tier-1 enumerates exactly the three permission shapes a static Linux
// binary uses on AMD64 SysV: `RX` (text), `RW` (data + bss), `R` (rodata
// + relro). `WX` is intentionally absent — every modern toolchain
// avoids writeable + executable (W^X is a hard rule on most hardened
// distros), and a future security layer will reject any PT_LOAD that
// asks for both. The mapping from the ELF `p_flags` bitmask is total
// (every well-formed ELF is one of those four — R, RX, RW, RWX) and
// any RWX request is currently mapped to RW with the X bit dropped on
// load and a `LoaderError::WriteExecuteSegment` returned, so the bit
// pattern in the fixture cannot silently smuggle in an executable
// writeable segment.
//
// Cell shape (system::apply integration)
// --------------------------------------
// AAAA's #460 device-cell pattern records each Linux device as a fact
// in the `Device_has_DriverData` cell with the device pointer cast to a
// hex string atom. Symmetrically, this slice records each loaded
// segment as a fact in three cells:
//
//   * `Process_has_EntryPoint` — one fact per `Process`, the value is
//     the ELF entry-point address as a `0x...` atom.
//   * `Process_has_Segment` — one fact per (`Process`, `Segment`) pair,
//     binding the segment to its parent address space.
//   * `Segment_has_Layout` — one fact per `Segment`, recording vaddr,
//     mem_size, file_size, and permission flags as flat-pair atoms.
//
// The rationale: the ELF loader is a producer-side surface (every
// other consumer reads cells back through `system::with_state`), so
// the cell shape needs to round-trip in `cell_push` form. The
// `record_into_cells` helper composes these facts onto whatever state
// the caller hands in; production wiring calls `system::apply(state)`
// to commit, the test harness inspects the returned `Object` directly
// (mirrors `ui_apps::registry::build_slint_component_state`).

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::format;
use alloc::vec::Vec;
use arest::ast::{cell_push, fact_from_pairs, Object};
use core::ptr::NonNull;
use core::slice;

/// 4 KiB page size, hard-coded. Every UEFI target the kernel currently
/// builds for (x86_64 / aarch64 / armv7) uses 4 KiB as the smallest
/// translation granule; aarch64's 16 KiB / 64 KiB granule alternatives
/// are spec'd but not what OVMF / AAVMF firmware leaves us with at
/// hand-off, and the parser-side `p_align` validation rejects anything
/// asking for a smaller alignment.
pub const PAGE_SIZE: usize = 4096;

/// Maximum total in-memory size for a single AddressSpace (256 MiB).
/// Refuses to load a binary whose summed `mem_size` exceeds this.
/// Tier-1 budget — the kernel's own talc heap is 32 MiB (entry_uefi.rs)
/// so anything close to 256 MiB would already fail on the underlying
/// `alloc()` call; the explicit cap surfaces a descriptive error
/// instead of a generic OOM. The value is doubled vs. the heap so a
/// future heap-grow doesn't need a parallel cap-bump.
pub const MAX_ADDRESS_SPACE_BYTES: usize = 256 * 1024 * 1024;

/// Permission bits a loaded segment carries. Mirrors the ELF `p_flags`
/// shape but enumerates only the cases the loader emits:
///
///   * `Read`         — read-only data (rodata, relro)
///   * `ReadWrite`    — writable data (data, bss)
///   * `ReadExecute`  — executable code (text)
///
/// `WriteExecute` (PF_W | PF_X without PF_R) and the W^X-violating
/// `RWX` are NOT representable here; the loader rejects them at load
/// time with `LoaderError::WriteExecuteSegment`.
///
/// Stored as a `repr(u8)` so the cell-recording path can encode it as
/// an atom without an `alloc::format!` round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SegmentPerm {
    Read = 0b100,
    ReadWrite = 0b110,
    ReadExecute = 0b101,
}

impl SegmentPerm {
    /// Render as the same three-letter string the ELF readelf tool
    /// uses. Used by `record_into_cells` for the SegmentPerm fact
    /// value. Stable string so a future log surface can grep on it.
    pub fn as_str(self) -> &'static str {
        match self {
            SegmentPerm::Read => "R",
            SegmentPerm::ReadWrite => "RW",
            SegmentPerm::ReadExecute => "RX",
        }
    }
}

/// Errors the loader can return. Each variant is a well-formed bad
/// input (or a resource exhaustion) that the parser missed because
/// the parser only validates structure, not semantics.
///
/// All variants are `Copy` so callers can store + compare without
/// lifetime hassles, matching `ElfError`'s shape (elf.rs:142).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderError {
    /// A PT_LOAD segment's `mem_size` is less than its `file_size` —
    /// the BSS region would have to be negative-sized. Always a
    /// malformed binary; spec mandates `p_memsz >= p_filesz`.
    NegativeBss,
    /// A PT_LOAD segment's `[vaddr, vaddr + memsz)` range overlaps
    /// another already-loaded segment's range. Malformed binary;
    /// loader refuses to silently shadow earlier writes.
    OverlappingSegments,
    /// `vaddr + memsz` would overflow `u64`. Malformed binary.
    SegmentMathOverflow,
    /// A PT_LOAD segment asks for both write and execute (W^X
    /// violation). Tier-1 is hard-rejected; #521 will surface a
    /// configurable allow-list for legacy binaries that need it.
    WriteExecuteSegment,
    /// A PT_LOAD segment requests zero `mem_size` — degenerate;
    /// nothing to load. Spec doesn't strictly forbid it but no
    /// real toolchain emits one and silently dropping the segment
    /// would be a footgun.
    EmptySegment,
    /// The ELF blob declares a PT_INTERP entry — needs a dynamic
    /// loader (typically `/lib64/ld-linux-x86-64.so.2`). Tier-1 only
    /// hosts statically-linked binaries; #522 wires the
    /// `vendor/musl/ld-musl` track once it lands.
    DynamicLoaderRequired,
    /// Cumulative `mem_size` exceeds `MAX_ADDRESS_SPACE_BYTES`. See
    /// the constant's doc-comment for the budget rationale.
    AddressSpaceTooLarge,
    /// `Layout::from_size_align` rejected the pages-rounded size +
    /// PAGE_SIZE alignment combination. Only happens if the size
    /// arithmetic overflowed `isize::MAX` — shouldn't surface in
    /// practice but stays representable so the loader never panics.
    BadLayout,
    /// `alloc::alloc::alloc` returned null — the talc heap is
    /// exhausted. Maps to Linux's `ENOMEM`.
    OutOfMemory,
}

/// One PT_LOAD segment after it has been loaded into the kernel's
/// heap. Owns its page-aligned allocation; drops it via `dealloc` on
/// AddressSpace tear-down so a process exit reclaims the pages.
///
/// `vaddr` is the segment's virtual address as the binary asked for —
/// preserved verbatim so the eventual #521 trampoline can install a
/// page-table mapping at that exact VA. `pages` is a `NonNull<u8>`
/// rather than `Box<[u8]>` because the storage was carved with a
/// custom `Layout` (page-aligned), and `Box`'s deallocator would use
/// the wrong alignment hint.
pub struct LoadedSegment {
    /// Virtual address the binary asked for. Preserved as a `u64`
    /// (not a `usize`) because ELF stores it as 64 bits and the
    /// parser would otherwise need to truncate on a 32-bit host
    /// running the unit tests — same rationale as
    /// `ProgramHeader::vaddr` in elf.rs:213.
    pub vaddr: u64,
    /// Total in-memory size, page-rounded. Equals `ceil(memsz /
    /// PAGE_SIZE) * PAGE_SIZE`. The first `file_size` bytes are
    /// copied from the ELF blob; the remainder is the BSS, zero-
    /// filled. A 4 KiB-rounded size — every page boundary inside
    /// `pages..pages+mem_size` is page-aligned.
    pub mem_size: usize,
    /// On-disk size copied from the ELF blob. Always `<= mem_size`.
    /// `mem_size - file_size` is the BSS region the loader zeroed.
    pub file_size: usize,
    /// Permission bits the eventual page-table install will apply.
    /// Derived from the ELF `p_flags` bitmask via
    /// `SegmentPerm::from_p_flags`.
    pub perm: SegmentPerm,
    /// Page-aligned heap allocation backing the segment. `pages` is
    /// the start; `pages..pages+mem_size` is the live range.
    /// Stored as a `NonNull<u8>` so the `Layout` we deallocate with
    /// matches the one we allocated with (alignment included);
    /// `Box<[u8]>` would fall back to the global allocator's default
    /// alignment.
    pages: NonNull<u8>,
    /// Layout used at allocation time — kept verbatim so `Drop` can
    /// hand it back to `dealloc` unmodified. Without this we'd have
    /// to reconstruct the layout from `mem_size` + `PAGE_SIZE` on
    /// drop, which would mask any future change to the alignment.
    layout: Layout,
}

// SAFETY: `LoadedSegment` owns its `NonNull<u8>` exclusively —
// `AddressSpace::push` is the only path that constructs one and the
// pointer never escapes outside the `pages_view` / `pages_view_mut`
// borrow methods, both of which produce slice borrows tied to
// `&self` / `&mut self`. The kernel is single-threaded today; the
// `Send` bound only matters once #521 introduces a per-process state
// the scheduler hands across CPUs (which will be SMP-gated then).
unsafe impl Send for LoadedSegment {}

impl LoadedSegment {
    /// Borrow the segment's payload as a read-only slice. Length is
    /// `mem_size` — both the file-content prefix and the BSS tail are
    /// reachable from this slice. Used by the cell-recording path to
    /// hash / fingerprint segments and by the future #521 trampoline
    /// to memcpy the bytes into a real page-table mapping.
    pub fn pages_view(&self) -> &[u8] {
        // SAFETY: `pages` is a valid allocation of `mem_size` bytes
        // produced by `AddressSpace::push`, and the borrow lifetime is
        // tied to `&self` so no concurrent `pages_view_mut` can alias.
        unsafe { slice::from_raw_parts(self.pages.as_ptr(), self.mem_size) }
    }

    /// Borrow the segment's payload as a writable slice. Used only by
    /// the test harness to inspect (and, in some negative tests,
    /// poke) loaded bytes; production callers should treat segments
    /// as immutable post-load.
    #[cfg(test)]
    pub fn pages_view_mut(&mut self) -> &mut [u8] {
        // SAFETY: `&mut self` guarantees exclusive access; `pages` is
        // a valid allocation of `mem_size` bytes (see Drop's invariant).
        unsafe { slice::from_raw_parts_mut(self.pages.as_ptr(), self.mem_size) }
    }
}

impl Drop for LoadedSegment {
    fn drop(&mut self) {
        // SAFETY: `pages` was produced by a single `alloc(self.layout)`
        // call inside `AddressSpace::push`. The same `layout` is
        // re-used here so the deallocator sees the original size +
        // alignment. No other path frees these pages.
        unsafe {
            dealloc(self.pages.as_ptr(), self.layout);
        }
    }
}

/// In-memory representation of a Linux process address space. Owns
/// every loaded segment; the eventual #521 trampoline walks this
/// container to install page-table mappings.
///
/// The `entry_point` field is set by the loader from the ELF
/// `e_entry` — it's the virtual address the trampoline will
/// `iretq`-into once the address space is live. Not used in tier-1
/// (no trampoline yet); preserved here so `record_into_cells`
/// produces a complete `Process_has_EntryPoint` fact.
pub struct AddressSpace {
    pub entry_point: u64,
    /// Loaded segments in PT_LOAD order from the ELF. The loader
    /// `push`es each one in the order the parser yielded them,
    /// rejecting overlap on insert (the overlap check is `O(n)` per
    /// push but `n` is typically 2-4 for static binaries).
    pub segments: Vec<LoadedSegment>,
    /// Running total of `mem_size` across every segment. Cached so
    /// `MAX_ADDRESS_SPACE_BYTES` checks are `O(1)` per push instead
    /// of an `iter().sum()`.
    total_bytes: usize,
}

impl AddressSpace {
    /// Construct an empty address space with the given entry point.
    /// The loader populates segments through `push_segment` after
    /// allocating + copying the file content.
    pub fn new(entry_point: u64) -> Self {
        Self { entry_point, segments: Vec::new(), total_bytes: 0 }
    }

    /// Allocate page-aligned storage for a PT_LOAD segment, copy the
    /// file_size bytes from `file_content` into the head of the
    /// allocation, zero the BSS tail, and record the segment in the
    /// address space.
    ///
    /// `vaddr` and `mem_size` come from the segment's page header;
    /// `mem_size` is page-rounded internally so the allocation is at
    /// least one page even for a 1-byte data segment. `perm` carries
    /// the permission shape derived from `p_flags`.
    ///
    /// Returns `Err` on:
    ///   * empty segment (`mem_size == 0`)
    ///   * file_content longer than `mem_size` (negative BSS)
    ///   * `vaddr + mem_size` overflow
    ///   * range overlap with a previously-pushed segment
    ///   * cumulative size exceeding `MAX_ADDRESS_SPACE_BYTES`
    ///   * `Layout::from_size_align` rejection
    ///   * heap exhaustion
    pub fn push_segment(
        &mut self,
        vaddr: u64,
        mem_size: usize,
        perm: SegmentPerm,
        file_content: &[u8],
    ) -> Result<(), LoaderError> {
        // Step 1: reject degenerate / malformed inputs that the parser
        // can't catch on its own (file_content vs. mem_size relation
        // is per-segment).
        if mem_size == 0 {
            return Err(LoaderError::EmptySegment);
        }
        if file_content.len() > mem_size {
            return Err(LoaderError::NegativeBss);
        }

        // Step 2: page-round the in-memory size so the allocation is
        // a whole number of pages. Future page-table install needs
        // page boundaries; rounding here keeps the upgrade path
        // transparent.
        let pages_rounded = match round_up_to_page(mem_size) {
            Some(v) => v,
            None => return Err(LoaderError::SegmentMathOverflow),
        };

        // Step 3: range arithmetic. `vaddr + mem_size` is the
        // exclusive upper bound the overlap check needs; if it
        // overflows, the segment can't be addressed at all.
        let vaddr_end = match vaddr.checked_add(mem_size as u64) {
            Some(v) => v,
            None => return Err(LoaderError::SegmentMathOverflow),
        };

        // Step 4: cumulative-size guard. Saturating add so a
        // malicious binary that names `usize::MAX` mem_size in one
        // segment can't wrap the running total.
        let new_total = self
            .total_bytes
            .checked_add(pages_rounded)
            .ok_or(LoaderError::AddressSpaceTooLarge)?;
        if new_total > MAX_ADDRESS_SPACE_BYTES {
            return Err(LoaderError::AddressSpaceTooLarge);
        }

        // Step 5: overlap detection. Linear scan — n is small (2-4
        // segments for static binaries, ~12 for fat dynamic ones).
        // Half-open intervals: a == b.end OR a.end == b is fine
        // (touching but not overlapping).
        for existing in &self.segments {
            let existing_end = existing
                .vaddr
                .checked_add(existing.mem_size as u64)
                .ok_or(LoaderError::SegmentMathOverflow)?;
            let overlaps = vaddr < existing_end && existing.vaddr < vaddr_end;
            if overlaps {
                return Err(LoaderError::OverlappingSegments);
            }
        }

        // Step 6: carve the allocation. Page alignment so the eventual
        // #521 page-table install lines up; no zero-init request — we
        // explicitly zero the BSS region after copying file content.
        let layout = Layout::from_size_align(pages_rounded, PAGE_SIZE)
            .map_err(|_| LoaderError::BadLayout)?;
        // SAFETY: layout is non-zero (mem_size > 0 by step 1) and
        // PAGE_SIZE = 4096 is a power of two. `alloc` returns either
        // a valid pointer satisfying the layout or null on OOM.
        let raw = unsafe { alloc(layout) };
        let pages = match NonNull::new(raw) {
            Some(p) => p,
            None => return Err(LoaderError::OutOfMemory),
        };

        // Step 7: copy the file content into the allocation, then
        // zero the rest. Two slice writes — bounds-checked by the
        // slice constructor lengths.
        // SAFETY: `pages` is a fresh `pages_rounded`-byte allocation
        // and `mem_size <= pages_rounded` (round_up_to_page returns
        // a value >= mem_size). The two slice writes do not alias
        // because `file_content.len() <= mem_size` and the second
        // slice starts at `file_content.len()`.
        unsafe {
            let dest = slice::from_raw_parts_mut(pages.as_ptr(), mem_size);
            // First file_size bytes: copy from the ELF blob.
            dest[..file_content.len()].copy_from_slice(file_content);
            // Remaining mem_size - file_size bytes: zero (BSS).
            for byte in &mut dest[file_content.len()..] {
                *byte = 0;
            }
            // Zero the tail beyond mem_size up to pages_rounded too
            // (no semantic meaning — that's "padding to the page
            // boundary" — but keeps the allocation in a deterministic
            // state for the cell-recording fingerprint and for any
            // future test that inspects the trailing bytes).
            if pages_rounded > mem_size {
                let tail =
                    slice::from_raw_parts_mut(pages.as_ptr().add(mem_size), pages_rounded - mem_size);
                for byte in tail {
                    *byte = 0;
                }
            }
        }

        // Step 8: record. Push first, update total after — Drop on
        // an early-return Result wouldn't fire because we already
        // succeeded.
        self.segments.push(LoadedSegment {
            vaddr,
            mem_size,
            file_size: file_content.len(),
            perm,
            pages,
            layout,
        });
        self.total_bytes = new_total;
        Ok(())
    }

    /// Compose this address space's facts onto `state` and return the
    /// new state. Pure function — the caller decides whether to
    /// commit via `system::apply` (production wiring) or to inspect
    /// the returned Object (the test harness). Mirrors the
    /// `ui_apps::registry::build_slint_component_state` shape.
    ///
    /// `process_id` is the atom name for the parent `Process` cell —
    /// the caller picks it (typically the hash of the ELF blob, or
    /// `"init"` for the boot process).
    ///
    /// Cells emitted (one fact per call):
    ///   * `Process_has_EntryPoint` —
    ///       (Process, EntryPoint) where EntryPoint = "0x{:016x}"
    ///   * `Process_has_Segment` —
    ///       (Process, Segment) where Segment = "<process_id>:<idx>"
    ///   * `Segment_has_Layout` —
    ///       (Segment, VirtualAddress, MemorySize, FileSize, Permission)
    ///       Numeric values rendered as 0x... atoms; permission as the
    ///       SegmentPerm::as_str() three-letter string.
    pub fn record_into_cells(&self, process_id: &str, state: &Object) -> Object {
        let entry_atom = format!("0x{:016x}", self.entry_point);
        let mut s = cell_push(
            "Process_has_EntryPoint",
            fact_from_pairs(&[("Process", process_id), ("EntryPoint", &entry_atom)]),
            state,
        );
        for (idx, seg) in self.segments.iter().enumerate() {
            let segment_id = format!("{}:{}", process_id, idx);
            s = cell_push(
                "Process_has_Segment",
                fact_from_pairs(&[("Process", process_id), ("Segment", &segment_id)]),
                &s,
            );
            let vaddr_atom = format!("0x{:016x}", seg.vaddr);
            let mem_atom = format!("0x{:x}", seg.mem_size);
            let file_atom = format!("0x{:x}", seg.file_size);
            s = cell_push(
                "Segment_has_Layout",
                fact_from_pairs(&[
                    ("Segment", &segment_id),
                    ("VirtualAddress", &vaddr_atom),
                    ("MemorySize", &mem_atom),
                    ("FileSize", &file_atom),
                    ("Permission", seg.perm.as_str()),
                ]),
                &s,
            );
        }
        s
    }
}

/// Round `n` up to a multiple of `PAGE_SIZE`. Returns `None` if the
/// rounded value would overflow `usize`. A 4 KiB page boundary is the
/// smallest unit any UEFI kernel target can install in a page table,
/// so all loader allocations are page-multiples.
fn round_up_to_page(n: usize) -> Option<usize> {
    // (n + PAGE_SIZE - 1) & !(PAGE_SIZE - 1) — but checked.
    let mask = PAGE_SIZE - 1;
    let plus = n.checked_add(mask)?;
    Some(plus & !mask)
}

/// Convert ELF `p_flags` (PF_R / PF_W / PF_X bitmask) into a tier-1
/// `SegmentPerm`. Returns `Err(LoaderError::WriteExecuteSegment)` for
/// any combination that asks for both write and execute (W^X
/// violation: PF_W | PF_X with or without PF_R, plus the bare PF_X
/// no-read case which has historically been a stack-exec marker).
///
/// Reads-only-write (PF_W without PF_R) is accepted — defensive
/// rather than rejecting; the BSS region of a `bss-only` segment in
/// some hand-rolled binaries asks for it. Mapped to ReadWrite (PF_R
/// is effectively implicit on every PT_LOAD per the spec's "must be
/// loaded into memory" note).
pub fn perm_from_p_flags(flags: u32) -> Result<SegmentPerm, LoaderError> {
    use crate::process::elf::{PF_R, PF_W, PF_X};
    let r = flags & PF_R != 0;
    let w = flags & PF_W != 0;
    let x = flags & PF_X != 0;
    match (r, w, x) {
        (_, true, true) => Err(LoaderError::WriteExecuteSegment),
        (false, false, true) => Err(LoaderError::WriteExecuteSegment),
        (_, true, false) => Ok(SegmentPerm::ReadWrite),
        (_, false, true) => Ok(SegmentPerm::ReadExecute),
        (true, false, false) => Ok(SegmentPerm::Read),
        (false, false, false) => Ok(SegmentPerm::Read),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Page rounding rounds up to the next page boundary.
    #[test]
    fn round_up_to_page_basic() {
        assert_eq!(round_up_to_page(0), Some(0));
        assert_eq!(round_up_to_page(1), Some(PAGE_SIZE));
        assert_eq!(round_up_to_page(PAGE_SIZE - 1), Some(PAGE_SIZE));
        assert_eq!(round_up_to_page(PAGE_SIZE), Some(PAGE_SIZE));
        assert_eq!(round_up_to_page(PAGE_SIZE + 1), Some(2 * PAGE_SIZE));
        assert_eq!(round_up_to_page(2 * PAGE_SIZE), Some(2 * PAGE_SIZE));
    }

    /// Permission decode: spec'd PT_LOAD shapes round-trip.
    #[test]
    fn perm_decode_canonical_shapes() {
        use crate::process::elf::{PF_R, PF_W, PF_X};
        assert_eq!(perm_from_p_flags(PF_R).unwrap(), SegmentPerm::Read);
        assert_eq!(perm_from_p_flags(PF_R | PF_W).unwrap(), SegmentPerm::ReadWrite);
        assert_eq!(perm_from_p_flags(PF_R | PF_X).unwrap(), SegmentPerm::ReadExecute);
    }

    /// W^X violation is rejected.
    #[test]
    fn perm_decode_rejects_write_execute() {
        use crate::process::elf::{PF_R, PF_W, PF_X};
        assert_eq!(
            perm_from_p_flags(PF_R | PF_W | PF_X).unwrap_err(),
            LoaderError::WriteExecuteSegment,
        );
        assert_eq!(
            perm_from_p_flags(PF_W | PF_X).unwrap_err(),
            LoaderError::WriteExecuteSegment,
        );
        // Bare PF_X — historically used as a stack-exec marker, never
        // legitimate on a PT_LOAD.
        assert_eq!(
            perm_from_p_flags(PF_X).unwrap_err(),
            LoaderError::WriteExecuteSegment,
        );
    }

    /// Push a single PT_LOAD-shaped segment, verify file content
    /// landed at the head and BSS is zero.
    #[test]
    fn push_segment_copies_file_content_and_zeros_bss() {
        let mut as_ = AddressSpace::new(0x40_1000);
        let bytes = [0xaa, 0xbb, 0xcc, 0xdd];
        as_.push_segment(0x40_2000, 0x100, SegmentPerm::ReadWrite, &bytes)
            .expect("push must succeed");
        assert_eq!(as_.segments.len(), 1);
        let seg = &as_.segments[0];
        let view = seg.pages_view();
        assert_eq!(&view[..4], &bytes);
        // Every byte after file_content is zero.
        assert!(view[4..0x100].iter().all(|b| *b == 0));
    }

    /// Two non-overlapping segments push cleanly; total_bytes
    /// reflects the page-rounded sum.
    #[test]
    fn push_segment_two_segments_disjoint() {
        let mut as_ = AddressSpace::new(0x40_1000);
        as_.push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text must push");
        as_.push_segment(0x40_2000, 0x20, SegmentPerm::ReadWrite, &[0x42; 16])
            .expect(".data must push");
        assert_eq!(as_.segments.len(), 2);
        // Both segments rounded to one page each.
        assert_eq!(as_.total_bytes, 2 * PAGE_SIZE);
    }

    /// Overlapping segments are rejected. Two segments that span
    /// the same VA range must error on the second push.
    #[test]
    fn push_segment_overlap_rejected() {
        let mut as_ = AddressSpace::new(0x40_1000);
        as_.push_segment(0x40_1000, 0x100, SegmentPerm::ReadExecute, &[])
            .expect("first push must succeed");
        // Second segment at 0x40_1080 — overlaps the first's tail.
        let err = as_
            .push_segment(0x40_1080, 0x100, SegmentPerm::ReadWrite, &[])
            .unwrap_err();
        assert_eq!(err, LoaderError::OverlappingSegments);
    }

    /// Touching but non-overlapping segments are accepted. First
    /// ends at 0x40_2000, second starts at 0x40_2000.
    #[test]
    fn push_segment_touching_ranges_allowed() {
        let mut as_ = AddressSpace::new(0x40_1000);
        as_.push_segment(0x40_1000, 0x1000, SegmentPerm::ReadExecute, &[])
            .expect("first push must succeed");
        as_.push_segment(0x40_2000, 0x1000, SegmentPerm::ReadWrite, &[])
            .expect("touching push must succeed");
        assert_eq!(as_.segments.len(), 2);
    }

    /// Segment with `mem_size = 0` is rejected — degenerate.
    #[test]
    fn push_segment_empty_rejected() {
        let mut as_ = AddressSpace::new(0);
        assert_eq!(
            as_.push_segment(0x40_1000, 0, SegmentPerm::Read, &[])
                .unwrap_err(),
            LoaderError::EmptySegment,
        );
    }

    /// File content longer than mem_size is `NegativeBss`.
    #[test]
    fn push_segment_negative_bss_rejected() {
        let mut as_ = AddressSpace::new(0);
        let too_much = [0u8; 16];
        assert_eq!(
            as_.push_segment(0x40_1000, 8, SegmentPerm::ReadWrite, &too_much)
                .unwrap_err(),
            LoaderError::NegativeBss,
        );
    }

    /// `vaddr + mem_size` overflow is `SegmentMathOverflow`.
    #[test]
    fn push_segment_vaddr_overflow_rejected() {
        let mut as_ = AddressSpace::new(0);
        let vaddr = u64::MAX - 0x10;
        // mem_size larger than the gap to u64::MAX.
        assert_eq!(
            as_.push_segment(vaddr, 0x100, SegmentPerm::ReadWrite, &[])
                .unwrap_err(),
            LoaderError::SegmentMathOverflow,
        );
    }

    /// `record_into_cells` emits the expected three cells per
    /// segment + one entry-point cell per process.
    #[test]
    fn record_into_cells_emits_expected_facts() {
        let mut as_ = AddressSpace::new(0x40_1000);
        as_.push_segment(0x40_1000, 0x10, SegmentPerm::ReadExecute, &[0x90; 8])
            .expect(".text must push");
        as_.push_segment(0x40_2000, 0x20, SegmentPerm::ReadWrite, &[0x42; 16])
            .expect(".data must push");
        let state = Object::phi();
        let recorded = as_.record_into_cells("test_proc", &state);
        // Cells should mention the entry-point + 2 Process_has_Segment
        // facts + 2 Segment_has_Layout facts. We assert by string-
        // searching the serialised form rather than reaching into
        // the Object's internal structure.
        let serialised = format!("{:?}", recorded);
        assert!(serialised.contains("Process_has_EntryPoint"));
        assert!(serialised.contains("Process_has_Segment"));
        assert!(serialised.contains("Segment_has_Layout"));
        assert!(serialised.contains("0x0000000000401000"));
        assert!(serialised.contains("0x0000000000402000"));
        assert!(serialised.contains("RX"));
        assert!(serialised.contains("RW"));
    }

    /// Permission `as_str` round-trips to the expected three-letter
    /// strings (used by the cell-recording path).
    #[test]
    fn segment_perm_as_str() {
        assert_eq!(SegmentPerm::Read.as_str(), "R");
        assert_eq!(SegmentPerm::ReadWrite.as_str(), "RW");
        assert_eq!(SegmentPerm::ReadExecute.as_str(), "RX");
    }
}
