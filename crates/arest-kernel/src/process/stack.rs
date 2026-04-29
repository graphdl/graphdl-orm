// crates/arest-kernel/src/process/stack.rs
//
// Initial-stack layout per System V AMD64 ABI for a freshly-spawned
// Linux process (#521, third slice of the #472 epic). Owns the page-
// aligned stack allocation and a builder that lays out argv / envp /
// auxv / argc on the stack exactly the way the ABI says, so when the
// trampoline (`process::trampoline`) jumps to `e_entry`, the C startup
// (`_start`) finds rsp pointing at argc with the rest of the arguments
// stacked above it.
//
// Why a builder pattern
// ---------------------
// The ABI's initial-stack layout has a strict ordering — argc at the
// lowest address, then argv (NULL-terminated), then envp (NULL-
// terminated), then auxv (an array of (key, value) pairs ending with
// AT_NULL), then the actual string data the argv/envp pointers refer
// to, then 16-byte alignment slack. Constructing the layout in three
// passes (count → reserve string area + 16-align → write pointers +
// strings) keeps the math local; a flat `prepare_stack(...)` function
// would mix all three concerns and make the alignment edge cases hard
// to read. Mirrors the shape of `crate::arch::uefi::memory::init` (also
// a multi-pass walk over a firmware-provided structure).
//
// Why we own the storage
// ----------------------
// The eventual #521 trampoline needs the stack to outlive the spawn
// call — it gets installed as the userspace `rsp` / `sp` value before
// the privilege transition, and the kernel must hold the storage for
// the lifetime of the process. `StackBuilder::finalize()` returns an
// owned `InitialStack` that the `Process` struct stores; drop reclaims
// the page allocation through the same `dealloc` path
// `LoadedSegment` uses (`process::address_space`).
//
// Why x86_64 only for the layout
// ------------------------------
// The auxv numeric constants (AT_RANDOM = 25, AT_PHDR = 3, etc.) are
// the same across all Linux architectures — they're defined in
// `<elf.h>` not `<asm-generic/elf.h>`. The initial-stack layout
// (argc, argv, envp, auxv, strings) is also architecture-agnostic per
// the System V ABI generic supplement. The 16-byte SP alignment
// requirement IS x86_64-specific (aarch64 uses 16-byte too, but
// armv7 uses 8-byte). The builder produces a layout that satisfies
// the strictest constraint (16-byte) so it's correct on every
// architecture the kernel currently targets; the per-arch trampoline
// is responsible for the actual privilege transition.
//
// Why argv[0] is the program path
// -------------------------------
// Linux convention. The first argv entry is the path the binary was
// invoked as (typically `/bin/sh` for a shell, `/bin/true` for true).
// For tier-1 we accept it as a builder argument so the call site
// records "this is what the ELF was loaded from" — even though there's
// no filesystem yet, the convention matters for the ELF's _start
// (some startups walk argv[0] to locate auxv).

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::vec::Vec;
use core::ptr::NonNull;
use core::slice;

use super::address_space::PAGE_SIZE;

/// Default initial stack size for a freshly-spawned process. 64 KiB
/// — large enough for argc + a typical argv (~8 entries × 256 bytes
/// each = 2 KiB), envp (~32 entries × 64 bytes each = 2 KiB), auxv
/// (~16 entries × 16 bytes each = 256 B), and the string area (~2 KiB),
/// plus comfortable headroom for the C startup's local frame. Linux
/// conventionally uses 8 MiB but the tier-1 budget caps the
/// `AddressSpace` itself at 256 MiB and we don't yet have a stack-
/// growth fault handler, so a smaller fixed allocation surfaces
/// stack-overflow as a clean #PF rather than an OOM during spawn.
pub const DEFAULT_STACK_SIZE: usize = 64 * 1024;

/// 16-byte stack alignment required by the System V AMD64 ABI at the
/// entry-point boundary. The first instruction of the loaded binary
/// (`_start`) assumes `(%rsp + 8) % 16 == 0` — i.e. that rsp is
/// 16-aligned BEFORE the implicit return address (which doesn't exist
/// on initial entry, hence the offset). The builder lays out the
/// stack so finalize-time rsp is exactly 16-aligned; the +8 offset
/// is the C startup's responsibility.
pub const STACK_ALIGN: usize = 16;

/// System V AMD64 PSABI auxiliary vector entry types. Values pulled
/// from `<elf.h>` (AUXV_TYPE constants) — these are stable across every
/// Linux architecture. A loader emits them so the C startup
/// (libc's `_start` / glibc's `__libc_start_main`) can locate process-
/// global resources without making syscalls. Tier-1 emits the minimum
/// set the loader knows up front; the runtime-derived ones (AT_UID /
/// AT_EUID / etc.) are skipped because they require a syscall surface
/// that doesn't exist yet (#473).
///
/// `repr(u64)` because `Elf64_auxv_t.a_type` is a `uint64_t` per the
/// Linux header. Not all variants are emitted — `auxv_layout` picks
/// just AT_PHDR / AT_PHENT / AT_PHNUM / AT_PAGESZ / AT_ENTRY /
/// AT_RANDOM / AT_NULL for the static-binary fast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum AuxvType {
    /// AT_NULL = 0. Terminator. Every auxv must end with one.
    Null = 0,
    /// AT_PHDR = 3. Address of the program-header table in the
    /// process address space. Normally `e_entry_segment_base + e_phoff`
    /// once the loader has chosen a base; tier-1 places phdrs at the
    /// PT_LOAD #0 base + ELF header offset.
    Phdr = 3,
    /// AT_PHENT = 4. Size of one program-header entry. Always 56 for
    /// ELF64 — see `super::elf::ELF64_PHENT_SIZE`.
    Phent = 4,
    /// AT_PHNUM = 5. Number of program-header entries.
    Phnum = 5,
    /// AT_PAGESZ = 6. System page size. Always 4096 — see
    /// `super::address_space::PAGE_SIZE`.
    Pagesz = 6,
    /// AT_ENTRY = 9. Process entry-point virtual address. Same value
    /// the trampoline jumps to — the C startup uses this to detect
    /// "I'm being invoked directly" vs. "via the dynamic linker."
    Entry = 9,
    /// AT_RANDOM = 25. Address of 16 bytes of CSPRNG output for libc's
    /// stack canary / pointer-mangle initialisation. The kernel
    /// supplies the bytes — for tier-1 we use a deterministic
    /// placeholder until #524's CSPRNG lands; libc tolerates any
    /// 16-byte value as long as it's stable for the process's
    /// lifetime.
    Random = 25,
}

/// One row of the auxiliary vector. `repr(C)` so the on-stack layout
/// matches `Elf64_auxv_t { uint64_t a_type; uint64_t a_val; }` —
/// libc's `__libc_start_main` walks the array as a flat `uint64_t[]`
/// of (type, value) pairs so the field order MUST match the C struct.
///
/// `Copy` because we accumulate them in a `Vec<AuxvEntry>` during
/// the builder pass and copy them to the stack in `finalize()`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct AuxvEntry {
    pub a_type: u64,
    pub a_val: u64,
}

impl AuxvEntry {
    /// Construct an entry with a typed key + raw u64 value. The
    /// `as u64` on `kind` is loss-free because `AuxvType` is `repr(u64)`.
    pub fn new(kind: AuxvType, val: u64) -> Self {
        Self { a_type: kind as u64, a_val: val }
    }
}

/// Errors the builder / finalize path can return. Stays `Copy` so a
/// caller can store + compare the variant without lifetime hassles
/// — same shape as `process::address_space::LoaderError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackError {
    /// Total payload (argc + pointers + auxv + strings) would not fit
    /// the stack page allocation. Tier-1 caps stacks at
    /// `DEFAULT_STACK_SIZE`; an over-large argv/envp will surface here.
    StackOverflow,
    /// `Layout::from_size_align` rejected the requested stack size +
    /// PAGE_SIZE alignment. Only happens if the size arithmetic
    /// overflowed `isize::MAX` — shouldn't surface in practice.
    BadLayout,
    /// `alloc::alloc::alloc` returned null on the stack-page request
    /// — the talc heap is exhausted. Maps to Linux's `ENOMEM`.
    OutOfMemory,
    /// String area math overflowed (extremely large argv/envp string
    /// sum). Defensive — tier-1 callers never hit this.
    StringAreaOverflow,
}

/// Owned initial stack — the page-aligned heap allocation plus the
/// final stack-pointer value the trampoline will load into rsp / sp.
///
/// Drops via the same `dealloc` + `Layout` path as
/// `process::address_space::LoadedSegment` so the storage reclaims
/// cleanly on `Process` tear-down.
#[derive(Debug)]
pub struct InitialStack {
    /// Page-aligned heap allocation backing the stack. The stack
    /// grows DOWN from `pages.add(size)` toward `pages.as_ptr()` —
    /// `sp_offset` records where the live data starts (everything from
    /// `pages.add(sp_offset)..pages.add(size)` is the populated
    /// initial frame; everything below is unused but reachable).
    pages: NonNull<u8>,
    /// Total in-memory size, page-rounded. Matches the
    /// `LoadedSegment` shape — page boundaries inside the allocation
    /// are page-aligned so the eventual page-table install can map
    /// each one with the right permissions.
    size: usize,
    /// Layout used at allocation time. Kept verbatim so `Drop` hands
    /// back exactly what was allocated (same alignment hint).
    layout: Layout,
    /// Offset within `pages` where the initial stack pointer lands.
    /// `pages.add(sp_offset)` is the value the trampoline loads into
    /// rsp / sp — argc lives at exactly that address, with argv /
    /// envp / auxv / strings stacked above it (toward higher
    /// addresses).
    sp_offset: usize,
    /// Top-of-stack offset (= `size`). Convenience for the trampoline
    /// — the userspace stack base for #PF reporting if the userspace
    /// program scribbles past its own stack frame.
    top_offset: usize,
}

// SAFETY: `InitialStack` owns its `NonNull<u8>` exclusively — only
// `StackBuilder::finalize` constructs one and the pointer never
// escapes outside the `view` borrow methods, which produce slice
// borrows tied to `&self` / `&mut self`. Mirrors `LoadedSegment`'s
// `Send` impl in `process::address_space`.
unsafe impl Send for InitialStack {}

impl InitialStack {
    /// The virtual / physical address the trampoline will load into
    /// `rsp` (x86_64) or `sp` (aarch64). Identity-mapped on UEFI so
    /// virt == phys.
    ///
    /// Returns the kernel-space pointer as a `u64` — when the
    /// trampoline #521 installs a real page-table and the stack maps
    /// to a userspace virtual range, this method will be augmented
    /// (or shadowed by an `sp_userspace`) to return the userspace VA;
    /// for tier-1 the kernel-space and userspace VAs coincide because
    /// no page table is yet active.
    pub fn sp(&self) -> u64 {
        // SAFETY: `pages` is a valid allocation of `size` bytes;
        // `sp_offset <= size` is enforced by `StackBuilder::finalize`.
        (unsafe { self.pages.as_ptr().add(self.sp_offset) }) as u64
    }

    /// Top of the stack (= base of the allocation + `size`). Useful
    /// for the trampoline's stack-bound reporting and for tests.
    pub fn top(&self) -> u64 {
        // SAFETY: same invariant as `sp` — `top_offset == size`.
        (unsafe { self.pages.as_ptr().add(self.top_offset) }) as u64
    }

    /// Total stack size (page-rounded). The trampoline reports this
    /// as the auxv `AT_PAGESZ`-rounded size for `sysconf(_SC_PAGESIZE)`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Borrow the entire stack region as a read-only slice. The
    /// populated initial frame lives in the tail
    /// (`stack[sp_offset..]`); the head (`stack[..sp_offset]`) is
    /// zero-initialised and reserved for the userspace program's
    /// own frames.
    pub fn view(&self) -> &[u8] {
        // SAFETY: `pages` is a valid allocation of `size` bytes; the
        // borrow lifetime is tied to `&self`.
        unsafe { slice::from_raw_parts(self.pages.as_ptr(), self.size) }
    }

    /// Borrow the entire stack region as a writable slice. Used only
    /// by the test harness to inspect (or, in some negative tests,
    /// poke) the initial frame; production callers should treat the
    /// stack as immutable post-finalize.
    #[cfg(test)]
    pub fn view_mut(&mut self) -> &mut [u8] {
        // SAFETY: `&mut self` guarantees exclusive access.
        unsafe { slice::from_raw_parts_mut(self.pages.as_ptr(), self.size) }
    }

    /// Borrow just the populated tail (the initial frame the
    /// trampoline hands to the userspace program). Equivalent to
    /// `&view()[sp_offset..]`. Convenience for the unit tests that
    /// assert the layout starts with argc, then NULL-terminated argv,
    /// etc.
    pub fn populated(&self) -> &[u8] {
        // SAFETY: `sp_offset <= size` by construction in `finalize`.
        unsafe {
            slice::from_raw_parts(
                self.pages.as_ptr().add(self.sp_offset),
                self.size - self.sp_offset,
            )
        }
    }

    /// Read the argc value from the populated frame. Convenience for
    /// tests; argc lives at the very start of the populated region by
    /// the System V layout.
    pub fn read_argc(&self) -> u64 {
        let pop = self.populated();
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&pop[..8]);
        u64::from_le_bytes(buf)
    }
}

impl Drop for InitialStack {
    fn drop(&mut self) {
        // SAFETY: `pages` was produced by a single `alloc(self.layout)`
        // call inside `StackBuilder::finalize`. The same `layout` is
        // re-used here so the deallocator sees the original size +
        // alignment.
        unsafe {
            dealloc(self.pages.as_ptr(), self.layout);
        }
    }
}

/// Builder for the initial stack frame. Three-stage usage:
///
///   1. `StackBuilder::new(top_addr)` — declare the upper bound (top
///      of the stack region; rsp grows DOWN from here).
///   2. `.push_argv(&[arg0, arg1, ...])` and `.push_envp(&[...])` —
///      record the argv / envp string lists. Order matters; the
///      builder lays them out in call order.
///   3. `.push_auxv(...)` — record the auxiliary vector entries.
///   4. `.finalize()` — allocate the stack page, lay out argc /
///      argv / envp / auxv / strings in System V order, and return
///      the `InitialStack` with the final rsp pointer.
///
/// The builder accumulates byte counts during the push phase; the
/// allocation happens once at `finalize` time so the layout sum is
/// known before any heap touch. This matches the shape of
/// `Vec::with_capacity` — declare intent, then commit.
///
/// Storage is owned by the returned `InitialStack`; the builder
/// itself holds nothing heap-backed once `finalize` consumes it.
pub struct StackBuilder<'a> {
    /// Argv strings in call order. Borrowed from the caller — the
    /// builder doesn't allocate string copies until `finalize` time
    /// when they get written into the stack region.
    argv: Vec<&'a [u8]>,
    /// Envp strings in call order. Same shape as `argv`.
    envp: Vec<&'a [u8]>,
    /// Auxv entries in declaration order. The terminator (AT_NULL)
    /// gets appended automatically by `finalize`; callers MUST NOT
    /// emit one explicitly.
    auxv: Vec<AuxvEntry>,
    /// Requested total stack size. Defaults to `DEFAULT_STACK_SIZE`;
    /// callers can opt into a larger allocation via
    /// `with_stack_size(...)` if a future use case needs it (tier-1
    /// has no caller that does).
    stack_size: usize,
}

impl<'a> StackBuilder<'a> {
    /// Construct a fresh builder. `stack_size` defaults to
    /// `DEFAULT_STACK_SIZE`; use `with_stack_size` to override.
    pub fn new() -> Self {
        Self {
            argv: Vec::new(),
            envp: Vec::new(),
            auxv: Vec::new(),
            stack_size: DEFAULT_STACK_SIZE,
        }
    }

    /// Override the stack size. Useful for tests that want a smaller
    /// allocation to fit a shrunk talc heap; tier-1 production never
    /// calls this (the default is sized for the C startup's frame).
    pub fn with_stack_size(mut self, size: usize) -> Self {
        self.stack_size = size;
        self
    }

    /// Append `arg` to the argv list. By convention `argv[0]` is the
    /// program path (e.g. `/bin/true`); subsequent entries are the
    /// command-line arguments. The builder takes a borrow — the
    /// string bytes must outlive the builder, but the resulting
    /// `InitialStack` carries its own copy so the borrows can drop
    /// after `finalize`.
    pub fn push_argv(mut self, arg: &'a [u8]) -> Self {
        self.argv.push(arg);
        self
    }

    /// Append `var` to the envp list. Convention: each entry is
    /// `KEY=value` with no trailing NUL (the builder appends NULs at
    /// finalize time per ABI). Order matches the order envp pointers
    /// land on the stack.
    pub fn push_envp(mut self, var: &'a [u8]) -> Self {
        self.envp.push(var);
        self
    }

    /// Append a single auxv entry. Callers MUST NOT emit AT_NULL —
    /// `finalize` appends the terminator automatically. Order matches
    /// the order auxv pairs land on the stack.
    pub fn push_auxv(mut self, entry: AuxvEntry) -> Self {
        self.auxv.push(entry);
        self
    }

    /// Allocate the stack region, lay out argc / argv-pointers /
    /// envp-pointers / auxv / strings per System V, and return the
    /// `InitialStack` with the populated rsp.
    ///
    /// Layout from low-address to high-address (rsp at the lowest):
    ///
    ///   ┌──────────────────────────┐  <-- rsp (16-aligned)
    ///   │ argc                     │  u64
    ///   │ argv[0]                  │  *u8  → string area
    ///   │ argv[1]                  │
    ///   │ ...                      │
    ///   │ argv[argc] = NULL        │  u64 = 0
    ///   │ envp[0]                  │  *u8  → string area
    ///   │ envp[1]                  │
    ///   │ ...                      │
    ///   │ envp[N] = NULL           │  u64 = 0
    ///   │ auxv[0].a_type           │  u64
    ///   │ auxv[0].a_val            │  u64
    ///   │ ...                      │
    ///   │ auxv[N].a_type = AT_NULL │  u64 = 0
    ///   │ auxv[N].a_val            │  u64 = 0  (ignored)
    ///   │ string area              │  argv + envp string bytes (NUL-
    ///   │                          │  terminated, packed)
    ///   │ alignment pad            │  zero-filled to 16-align rsp
    ///   └──────────────────────────┘  <-- top (= stack page base + size)
    ///
    /// The 16-byte alignment is satisfied by adjusting the boundary
    /// between the auxv section and the string area: every auxv pair
    /// is 16 bytes, so the auxv alignment is automatic; the string
    /// area's start is 8-byte-aligned naturally; the END of the string
    /// area is the top of the stack and might not be 16-aligned, so
    /// we round-up the string-area size to satisfy the constraint.
    pub fn finalize(self) -> Result<InitialStack, StackError> {
        // Step 1: tally string-area bytes (argv strings + envp strings,
        // each NUL-terminated).
        let argv_str_bytes = self
            .argv
            .iter()
            .try_fold(0usize, |acc, s| acc.checked_add(s.len() + 1))
            .ok_or(StackError::StringAreaOverflow)?;
        let envp_str_bytes = self
            .envp
            .iter()
            .try_fold(0usize, |acc, s| acc.checked_add(s.len() + 1))
            .ok_or(StackError::StringAreaOverflow)?;
        let string_area_bytes = argv_str_bytes
            .checked_add(envp_str_bytes)
            .ok_or(StackError::StringAreaOverflow)?;

        // Step 2: tally pointer-area bytes. Counts (each is 8 bytes):
        //   * 1   for argc
        //   * argc.len() + 1   for argv pointers + NULL terminator
        //   * envp.len() + 1   for envp pointers + NULL terminator
        //   * (auxv.len() + 1) * 2  for (type, val) pairs + AT_NULL
        let pointer_count = 1
            + self.argv.len() + 1
            + self.envp.len() + 1
            + (self.auxv.len() + 1) * 2;
        let pointer_area_bytes = pointer_count
            .checked_mul(8)
            .ok_or(StackError::StackOverflow)?;

        // Step 3: total payload + 16-byte alignment slack.
        let payload_bytes = pointer_area_bytes
            .checked_add(string_area_bytes)
            .ok_or(StackError::StackOverflow)?;
        // Round payload up to 16 bytes so the rsp value lands on a
        // 16-byte boundary at the LOW end of the populated region.
        let payload_aligned = round_up(payload_bytes, STACK_ALIGN)
            .ok_or(StackError::StackOverflow)?;
        if payload_aligned > self.stack_size {
            return Err(StackError::StackOverflow);
        }

        // Step 4: allocate the stack page. Page-aligned so a future
        // page-table install lines up; size == self.stack_size
        // (which the caller picked, defaulting to DEFAULT_STACK_SIZE).
        let stack_size_aligned = round_up(self.stack_size, PAGE_SIZE)
            .ok_or(StackError::BadLayout)?;
        let layout = Layout::from_size_align(stack_size_aligned, PAGE_SIZE)
            .map_err(|_| StackError::BadLayout)?;
        // SAFETY: layout is non-zero (stack_size_aligned > 0 because
        // payload_aligned > 0 and that already rounded up), and
        // PAGE_SIZE is a power of two. `alloc` returns either a valid
        // pointer or null on OOM.
        let raw = unsafe { alloc(layout) };
        let pages = NonNull::new(raw).ok_or(StackError::OutOfMemory)?;

        // Zero-initialise the entire allocation so the head of the
        // stack (everything below sp_offset) is deterministic. Defends
        // against an info-leak from kernel heap pages into userspace.
        // SAFETY: fresh allocation of `stack_size_aligned` bytes.
        unsafe {
            core::ptr::write_bytes(pages.as_ptr(), 0, stack_size_aligned);
        }

        // Step 5: compute sp_offset. The populated region occupies
        // `[stack_size_aligned - payload_aligned, stack_size_aligned)`
        // — sp = base + (stack_size_aligned - payload_aligned).
        let sp_offset = stack_size_aligned - payload_aligned;

        // Step 6: lay out the populated region. We write at byte
        // offsets from `sp_offset` (= the future rsp value).
        //
        // Cursor `cursor_ptr` walks forward through the pointer area;
        // cursor `string_ptr` walks forward through the string area.
        // The string area starts at `sp_offset + pointer_area_bytes`.
        let pointer_area_base = sp_offset;
        let string_area_base = sp_offset + pointer_area_bytes;

        // SAFETY for the rest of this function: every offset arithmetic
        // is bounded by `stack_size_aligned` because `payload_aligned <=
        // stack_size_aligned` (checked above) and `payload_bytes <=
        // payload_aligned`. The writes don't overlap because the
        // pointer area ends exactly where the string area begins, and
        // the string area is sized to fit `string_area_bytes` exactly
        // (the alignment slack is on the high-address side, beyond the
        // last string byte).
        let mut cursor: usize = pointer_area_base;
        let mut string_cursor: usize = string_area_base;

        // 6a: argc (1 × u64).
        let argc = self.argv.len() as u64;
        unsafe {
            write_u64_le(pages.as_ptr().add(cursor), argc);
        }
        cursor += 8;

        // 6b: argv pointers + NULL terminator. Each pointer is the
        // absolute address of the string in the stack region.
        for arg in &self.argv {
            let str_addr = (unsafe { pages.as_ptr().add(string_cursor) }) as u64;
            unsafe {
                write_u64_le(pages.as_ptr().add(cursor), str_addr);
            }
            cursor += 8;
            // Copy the string bytes + NUL terminator into the string area.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    arg.as_ptr(),
                    pages.as_ptr().add(string_cursor),
                    arg.len(),
                );
                // NUL terminator was already zeroed by the bulk write_bytes
                // at allocation time, but write it explicitly for clarity.
                *pages.as_ptr().add(string_cursor + arg.len()) = 0;
            }
            string_cursor += arg.len() + 1;
        }
        // NULL-terminate the argv array.
        unsafe {
            write_u64_le(pages.as_ptr().add(cursor), 0);
        }
        cursor += 8;

        // 6c: envp pointers + NULL terminator. Same shape as argv.
        for var in &self.envp {
            let str_addr = (unsafe { pages.as_ptr().add(string_cursor) }) as u64;
            unsafe {
                write_u64_le(pages.as_ptr().add(cursor), str_addr);
            }
            cursor += 8;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    var.as_ptr(),
                    pages.as_ptr().add(string_cursor),
                    var.len(),
                );
                *pages.as_ptr().add(string_cursor + var.len()) = 0;
            }
            string_cursor += var.len() + 1;
        }
        unsafe {
            write_u64_le(pages.as_ptr().add(cursor), 0);
        }
        cursor += 8;

        // 6d: auxv entries + AT_NULL terminator. Each entry is two
        // u64s: a_type then a_val.
        for entry in &self.auxv {
            unsafe {
                write_u64_le(pages.as_ptr().add(cursor), entry.a_type);
                write_u64_le(pages.as_ptr().add(cursor + 8), entry.a_val);
            }
            cursor += 16;
        }
        // Append AT_NULL terminator.
        unsafe {
            write_u64_le(pages.as_ptr().add(cursor), AuxvType::Null as u64);
            write_u64_le(pages.as_ptr().add(cursor + 8), 0);
        }
        cursor += 16;

        // Sanity: cursor ends exactly at string_area_base.
        debug_assert_eq!(
            cursor, string_area_base,
            "pointer-area cursor must land on the string-area base — \
             pointer-area sizing is wrong"
        );

        Ok(InitialStack {
            pages,
            size: stack_size_aligned,
            layout,
            sp_offset,
            top_offset: stack_size_aligned,
        })
    }
}

impl<'a> Default for StackBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Round `n` up to a multiple of `align` (which must be a power of
/// two). Returns `None` on overflow. Used for both 16-byte rsp
/// alignment and 4-KiB page alignment.
fn round_up(n: usize, align: usize) -> Option<usize> {
    debug_assert!(align.is_power_of_two(), "align must be a power of two");
    let mask = align - 1;
    n.checked_add(mask).map(|v| v & !mask)
}

/// Write a `u64` to `dst` in little-endian order. Equivalent to
/// `dst.cast::<u64>().write_unaligned(v.to_le())` but spelled out so
/// the intent is obvious to a reader. The trampoline asm reads these
/// values via the System V ABI's argc / argv access pattern, which
/// expects native-endian — and x86_64 / aarch64 are both LE.
///
/// SAFETY: caller must guarantee `dst` is valid for 8 bytes of write.
unsafe fn write_u64_le(dst: *mut u8, v: u64) {
    let bytes = v.to_le_bytes();
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, 8);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-up basic shapes. Power-of-two alignments only.
    #[test]
    fn round_up_basic() {
        assert_eq!(round_up(0, 16), Some(0));
        assert_eq!(round_up(1, 16), Some(16));
        assert_eq!(round_up(15, 16), Some(16));
        assert_eq!(round_up(16, 16), Some(16));
        assert_eq!(round_up(17, 16), Some(32));
        assert_eq!(round_up(0, 4096), Some(0));
        assert_eq!(round_up(1, 4096), Some(4096));
    }

    /// Empty stack (no argv, envp, or auxv) — argc = 0, argv NULL,
    /// envp NULL, auxv AT_NULL. Smallest possible payload.
    #[test]
    fn finalize_empty_stack_layout() {
        let stack = StackBuilder::new()
            .finalize()
            .expect("empty finalize must succeed");
        // argc lives at sp.
        assert_eq!(stack.read_argc(), 0);
        // The populated region: argc(8) + argv_null(8) + envp_null(8)
        // + auxv_null(16) = 40 bytes, padded to 48 for 16-align.
        let pop = stack.populated();
        assert_eq!(pop.len(), 48);
        // Bytes 8..16: argv NULL.
        assert_eq!(read_u64_le(&pop[8..16]), 0);
        // Bytes 16..24: envp NULL.
        assert_eq!(read_u64_le(&pop[16..24]), 0);
        // Bytes 24..32: auxv[0].a_type = AT_NULL.
        assert_eq!(read_u64_le(&pop[24..32]), AuxvType::Null as u64);
        // Bytes 32..40: auxv[0].a_val = 0.
        assert_eq!(read_u64_le(&pop[32..40]), 0);
    }

    /// SP is 16-aligned per System V ABI.
    #[test]
    fn finalize_sp_is_16_aligned() {
        let stack = StackBuilder::new()
            .push_argv(b"/bin/true")
            .push_envp(b"PATH=/usr/bin")
            .finalize()
            .expect("finalize must succeed");
        assert_eq!(stack.sp() % 16, 0, "sp must be 16-aligned");
    }

    /// argc reflects the number of argv entries.
    #[test]
    fn finalize_argc_matches_argv_count() {
        let stack = StackBuilder::new()
            .push_argv(b"/bin/sh")
            .push_argv(b"-c")
            .push_argv(b"echo hi")
            .finalize()
            .expect("finalize must succeed");
        assert_eq!(stack.read_argc(), 3);
    }

    /// argv pointers point at NUL-terminated strings inside the
    /// stack region. Round-trip the first argv entry.
    #[test]
    fn finalize_argv_pointers_resolve_to_strings() {
        let stack = StackBuilder::new()
            .push_argv(b"/bin/true")
            .finalize()
            .expect("finalize must succeed");
        let pop = stack.populated();
        // argc (8) + argv[0] (8) at offset 8.
        let argv0_ptr = read_u64_le(&pop[8..16]);
        // The pointer is an absolute address; subtract the stack
        // base to get an offset into `view()`.
        let view_base = stack.view().as_ptr() as u64;
        let argv0_offset = (argv0_ptr - view_base) as usize;
        let view = stack.view();
        // Read 9 bytes ("/bin/true") + the trailing NUL.
        assert_eq!(&view[argv0_offset..argv0_offset + 9], b"/bin/true");
        assert_eq!(view[argv0_offset + 9], 0);
    }

    /// envp pointers point at NUL-terminated strings inside the
    /// stack region. Round-trip the first envp entry.
    #[test]
    fn finalize_envp_pointers_resolve_to_strings() {
        let stack = StackBuilder::new()
            .push_envp(b"PATH=/usr/bin")
            .finalize()
            .expect("finalize must succeed");
        let pop = stack.populated();
        // argc (8) + argv NULL (8) + envp[0] (8) at offset 16.
        let envp0_ptr = read_u64_le(&pop[16..24]);
        let view_base = stack.view().as_ptr() as u64;
        let envp0_offset = (envp0_ptr - view_base) as usize;
        let view = stack.view();
        assert_eq!(&view[envp0_offset..envp0_offset + 13], b"PATH=/usr/bin");
        assert_eq!(view[envp0_offset + 13], 0);
    }

    /// argv NULL terminator follows the last argv pointer.
    #[test]
    fn finalize_argv_null_terminator() {
        let stack = StackBuilder::new()
            .push_argv(b"/bin/sh")
            .push_argv(b"-c")
            .finalize()
            .expect("finalize must succeed");
        let pop = stack.populated();
        // argc (8) + argv[0] (8) + argv[1] (8) + argv[2] = NULL at offset 24.
        assert_eq!(read_u64_le(&pop[24..32]), 0, "argv must end with NULL");
    }

    /// envp NULL terminator follows the last envp pointer.
    #[test]
    fn finalize_envp_null_terminator() {
        let stack = StackBuilder::new()
            .push_envp(b"PATH=/usr/bin")
            .push_envp(b"HOME=/root")
            .finalize()
            .expect("finalize must succeed");
        let pop = stack.populated();
        // argc (8) + argv NULL (8) + envp[0] (8) + envp[1] (8) +
        // envp[2] = NULL at offset 32.
        assert_eq!(read_u64_le(&pop[32..40]), 0, "envp must end with NULL");
    }

    /// auxv entries land in declaration order with AT_NULL terminator.
    #[test]
    fn finalize_auxv_layout_with_terminator() {
        let stack = StackBuilder::new()
            .push_auxv(AuxvEntry::new(AuxvType::Pagesz, 4096))
            .push_auxv(AuxvEntry::new(AuxvType::Entry, 0x40_1000))
            .finalize()
            .expect("finalize must succeed");
        let pop = stack.populated();
        // argc (8) + argv NULL (8) + envp NULL (8) = offset 24.
        // First auxv pair at offset 24..40.
        assert_eq!(read_u64_le(&pop[24..32]), AuxvType::Pagesz as u64);
        assert_eq!(read_u64_le(&pop[32..40]), 4096);
        // Second auxv pair at offset 40..56.
        assert_eq!(read_u64_le(&pop[40..48]), AuxvType::Entry as u64);
        assert_eq!(read_u64_le(&pop[48..56]), 0x40_1000);
        // AT_NULL terminator at offset 56..72.
        assert_eq!(read_u64_le(&pop[56..64]), AuxvType::Null as u64);
        assert_eq!(read_u64_le(&pop[64..72]), 0);
    }

    /// AuxvEntry's repr is (type, val) in field order.
    #[test]
    fn auxv_entry_field_order() {
        let entry = AuxvEntry::new(AuxvType::Pagesz, 4096);
        assert_eq!(entry.a_type, 6);
        assert_eq!(entry.a_val, 4096);
    }

    /// AuxvType numeric values match the spec.
    #[test]
    fn auxv_type_numeric_values() {
        assert_eq!(AuxvType::Null as u64, 0);
        assert_eq!(AuxvType::Phdr as u64, 3);
        assert_eq!(AuxvType::Phent as u64, 4);
        assert_eq!(AuxvType::Phnum as u64, 5);
        assert_eq!(AuxvType::Pagesz as u64, 6);
        assert_eq!(AuxvType::Entry as u64, 9);
        assert_eq!(AuxvType::Random as u64, 25);
    }

    /// Stack size respected — the allocation rounds to the next page.
    #[test]
    fn finalize_stack_size_page_rounded() {
        let stack = StackBuilder::new()
            .with_stack_size(8192)
            .finalize()
            .expect("finalize must succeed");
        assert_eq!(stack.size(), 8192);
    }

    /// `top` returns the high-end of the allocation (= base + size).
    #[test]
    fn finalize_top_at_allocation_end() {
        let stack = StackBuilder::new()
            .with_stack_size(8192)
            .finalize()
            .expect("finalize must succeed");
        let view_base = stack.view().as_ptr() as u64;
        assert_eq!(stack.top(), view_base + 8192);
    }

    /// SP < top (stack grows DOWN — sp is below top).
    #[test]
    fn finalize_sp_below_top() {
        let stack = StackBuilder::new()
            .push_argv(b"/bin/true")
            .finalize()
            .expect("finalize must succeed");
        assert!(stack.sp() < stack.top(), "sp must be < top (stack grows down)");
    }

    /// Stack overflow returns the typed error rather than panicking
    /// or silently truncating.
    #[test]
    fn finalize_too_small_stack_overflows() {
        // Cap at 256 bytes — much smaller than the smallest possible
        // payload with a typical-sized argv. With an absurdly long
        // argv, exceed the cap.
        let huge_arg = [b'x'; 1024];
        let err = StackBuilder::new()
            .with_stack_size(256)
            .push_argv(&huge_arg)
            .finalize()
            .unwrap_err();
        assert_eq!(err, StackError::StackOverflow);
    }

    /// Drop reclaims the stack page (no leak / no double-free). Best
    /// proxy: construct + drop in a tight loop and ensure the heap
    /// doesn't blow up. talc has no public hooks for "current usage"
    /// so this is structural — if Drop weren't wired, `dealloc`
    /// wouldn't run.
    #[test]
    fn drop_is_called_no_panic() {
        for _ in 0..16 {
            let stack = StackBuilder::new()
                .push_argv(b"/bin/true")
                .finalize()
                .expect("finalize must succeed");
            drop(stack);
        }
    }

    /// Combined stress: argv + envp + full auxv, all the bells and
    /// whistles. Verify SP is 16-aligned and argc reads back.
    #[test]
    fn finalize_full_combo_layout() {
        let random_addr = 0xDEAD_BEEF_0000_0000u64;
        let phdr_addr = 0x0040_0040u64;
        let entry_addr = 0x0040_1000u64;
        let stack = StackBuilder::new()
            .push_argv(b"/bin/sh")
            .push_argv(b"-c")
            .push_argv(b"echo hi")
            .push_envp(b"PATH=/usr/bin")
            .push_envp(b"HOME=/root")
            .push_envp(b"LANG=C.UTF-8")
            .push_auxv(AuxvEntry::new(AuxvType::Phdr, phdr_addr))
            .push_auxv(AuxvEntry::new(AuxvType::Phent, 56))
            .push_auxv(AuxvEntry::new(AuxvType::Phnum, 2))
            .push_auxv(AuxvEntry::new(AuxvType::Pagesz, 4096))
            .push_auxv(AuxvEntry::new(AuxvType::Entry, entry_addr))
            .push_auxv(AuxvEntry::new(AuxvType::Random, random_addr))
            .finalize()
            .expect("combo finalize must succeed");
        assert_eq!(stack.sp() % 16, 0);
        assert_eq!(stack.read_argc(), 3);
    }

    // -- Helper for the test bodies ----------------------------------

    /// Read a little-endian `u64` out of a slice. Mirrors the on-stack
    /// write that `write_u64_le` does. Fails if `s.len() < 8`.
    fn read_u64_le(s: &[u8]) -> u64 {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&s[..8]);
        u64::from_le_bytes(buf)
    }
}
