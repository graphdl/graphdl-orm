// crates/arest-kernel/src/process/elf.rs
//
// ELF64 file-header + program-header parser (#518, first slice of
// #472's Linux process spawn epic). Pure parse, no memory mapping —
// downstream slices (#472b loader, #472c relocations, #472d dynamic
// linker) consume the `ParsedElf` produced here.
//
// Why hand-rolled instead of `goblin`
// -----------------------------------
// goblin is the obvious choice for ELF parsing in the Rust ecosystem,
// but it does not build cleanly under no_std without a flurry of
// feature-flag selection (`elf64`, `endian_fd`, `alloc`) and even then
// pulls in `scroll` which carries a `std` shadow under
// `no_default_features`. The parsing surface we need is tiny — one
// 64-byte file header and N × 56-byte program-header entries, all
// little-endian — and the cost of vendoring ~200 lines is much less
// than the carrying cost of bisecting goblin upgrades against the
// kernel's nightly pin (rust-toolchain.toml).
//
// What we model and what we don't
// --------------------------------
// Modelled (this slice):
//   * ELF64 file header validation: magic / class / endianness /
//     machine / OS-ABI / type. Everything the loader needs to refuse
//     a bad input *before* trying to interpret a phdr table.
//   * Program-header table walk: enumerate every entry, classify it
//     as PT_LOAD / PT_INTERP / PT_GNU_STACK / PT_TLS / Other. Bounds-
//     check the table against the file size so a truncated input
//     can't read past `bytes.len()`.
//
// Not modelled (future slices):
//   * Section header table — irrelevant to loading; only used by
//     debuggers / linkers.
//   * Dynamic-linking metadata (PT_DYNAMIC interpretation) — the
//     foundation slice supports static binaries; PIE / dynamic comes
//     in #472d.
//   * Memory mapping into a process address space — #472b.
//   * Relocations — #472c.
//
// Endian discipline
// -----------------
// AMD64 SysV ABI mandates ELFDATA2LSB (little-endian). We refuse any
// other byte ordering at the file-header check, then read every multi-
// byte field via `u16::from_le_bytes` / `u32::from_le_bytes` /
// `u64::from_le_bytes` against the validated slice. No unsafe pointer
// casts — the input may be unaligned (downloaded via HTTP into a
// `Vec<u8>` that doesn't carry any alignment promise) and an aligned
// `*const ElfHeader` reinterpret would invoke UB on misaligned reads.
//
// Error model
// -----------
// `parse(...)` returns a fully-typed `Result<ParsedElf, ElfError>`.
// Every failure mode that can come from a malformed input is its own
// variant — callers can branch on which check failed (loader log
// surface; HTTP "bad upload" rejection later) without string-matching.
// No `panic!` on any input path. Truncated tables / out-of-bounds
// program-header offsets surface as `ElfError::Truncated`; bad-magic /
// wrong-class / wrong-endian / wrong-machine / wrong-abi each get
// their own variant.

use alloc::vec::Vec;

use super::address_space::{perm_from_p_flags, AddressSpace, LoaderError};

// -- Constants from the ELF64 spec ----------------------------------
//
// Pulled directly from the System V ABI Edition 4.1 + AMD64 PSABI
// supplement. Numeric literals are normative — the spec specifies
// them as exactly these values, so no #[non_exhaustive] dance.

/// `\x7fELF` magic bytes. Every ELF file starts with these four.
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// `e_ident[EI_CLASS] = ELFCLASS64`. We refuse 32-bit ELF (ELFCLASS32 = 1).
pub const ELFCLASS64: u8 = 2;

/// `e_ident[EI_DATA] = ELFDATA2LSB`. We refuse big-endian (ELFDATA2MSB = 2).
pub const ELFDATA2LSB: u8 = 1;

/// `e_ident[EI_VERSION] = EV_CURRENT`. Spec requires this be 1 for any
/// valid file; gates a future `EV_NONE = 0` rejection too.
pub const EV_CURRENT: u8 = 1;

/// `e_ident[EI_OSABI] = ELFOSABI_SYSV (0)`. Linux toolchains historically
/// emit SYSV; ELFOSABI_LINUX (3) appeared with `gnu` extensions. We
/// accept both — the kernel is happy to host either.
pub const ELFOSABI_SYSV: u8 = 0;
pub const ELFOSABI_LINUX: u8 = 3;

/// Size of `e_ident` in bytes — fixed by spec. The first 16 bytes of
/// every ELF file are the ident block.
pub const EI_NIDENT: usize = 16;

/// `e_type` values we accept. Static binaries are ET_EXEC (2);
/// position-independent executables (PIE — gcc -fPIE) are emitted as
/// ET_DYN (3) and run identically once relocations are applied (#472c).
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;

/// `e_machine = EM_X86_64`. Spec value 62 (= 0x3e). The kernel's
/// initial Linux-binary support is amd64-only; aarch64 (`EM_AARCH64`)
/// support follows once the kernel grows an aarch64 process model.
pub const EM_X86_64: u16 = 62;

/// `p_type` values we recognise. PT_LOAD is the workhorse — every
/// loadable segment. PT_INTERP names the dynamic linker (`/lib64/ld-
/// linux-x86-64.so.2`). PT_GNU_STACK is a permissions hint for the
/// stack. PT_TLS describes thread-local storage. Everything else
/// (PT_DYNAMIC, PT_NOTE, PT_PHDR, PT_GNU_RELRO, ...) classifies as
/// `SegmentKind::Other` — the loader can ignore them on the static-
/// binary path. Numeric values from elf.h.
pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_NOTE: u32 = 4;
pub const PT_SHLIB: u32 = 5;
pub const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;
pub const PT_GNU_STACK: u32 = 0x6474_e551;

/// `p_flags` bit definitions — readable / writable / executable. Used
/// when the loader builds page-table flags (#472b).
pub const PF_X: u32 = 0x1;
pub const PF_W: u32 = 0x2;
pub const PF_R: u32 = 0x4;

/// On-disk ELF64 file-header size in bytes. Hard-coded per spec —
/// validated against the file's own `e_ehsize` for sanity. If a future
/// extension grows the header, that's a spec break and we'd reject it.
pub const ELF64_HEADER_SIZE: usize = 64;

/// On-disk ELF64 program-header entry size in bytes. Same situation
/// as `ELF64_HEADER_SIZE`.
pub const ELF64_PHENT_SIZE: usize = 56;

// -- Errors ---------------------------------------------------------

/// Every distinguishable failure mode the parser can surface. Stays
/// `Copy` so callers can store it without lifetime hassles. No
/// `Display` impl yet — the loader surface will format these into a
/// log message at the call site once #472b lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfError {
    /// Input is shorter than the 64-byte file header — can't even
    /// read the magic. Includes truncated program-header tables and
    /// program headers that point past `bytes.len()`.
    Truncated,
    /// First four bytes are not `\x7fELF`. The most common rejection
    /// for "this isn't actually an ELF file."
    BadMagic,
    /// `e_ident[EI_CLASS]` is not ELFCLASS64. We don't (yet) support
    /// 32-bit Linux binaries; that's a parallel epic.
    WrongClass,
    /// `e_ident[EI_DATA]` is not ELFDATA2LSB. AMD64 is little-endian
    /// only per the PSABI.
    WrongEndian,
    /// `e_ident[EI_VERSION]` is not EV_CURRENT (1). Spec leaves room
    /// for a future EV_2 but no toolchain emits it; reject for now.
    WrongIdentVersion,
    /// `e_ident[EI_OSABI]` is neither SYSV (0) nor Linux (3). FreeBSD
    /// (9) and OpenBSD (12) binaries would land here — different ABI,
    /// different syscall interface, can't host them.
    WrongAbi,
    /// `e_type` is not ET_EXEC or ET_DYN. ET_REL (1, relocatable
    /// objects), ET_CORE (4, core dumps) lands here.
    WrongType,
    /// `e_machine` is not EM_X86_64. EM_AARCH64 (183) lands here for
    /// now — AAVMF/aarch64 process support is a future track.
    WrongMachine,
    /// `e_ehsize` does not match `ELF64_HEADER_SIZE`. Indicates a
    /// non-standard ELF flavour we don't recognise.
    BadHeaderSize,
    /// `e_phentsize` does not match `ELF64_PHENT_SIZE`. Same shape as
    /// `BadHeaderSize`.
    BadPhentSize,
    /// `e_phnum` × `e_phentsize` would overflow `usize` arithmetic
    /// before the bounds check, OR the table extends past the end of
    /// the input. Either is a malformed file.
    PhdrTableOverflow,
    /// A program header entry's `p_offset + p_filesz` would overflow
    /// or extends past the end of the input slice.
    SegmentOutOfBounds,
}

// -- Domain types ---------------------------------------------------

/// Classification of a single program header. The loader (#472b) will
/// dispatch on this — `Load` segments map into the address space,
/// `Interp` triggers the dynamic-linker path (#472d), `GnuStack`
/// records non-exec-stack permissions, `Tls` carves the TLS template,
/// `Other` is preserved for diagnostic display only.
///
/// Cheap to copy; no allocations behind any variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Load,
    Interp,
    GnuStack,
    Tls,
    /// Catch-all for PT_NULL, PT_DYNAMIC, PT_NOTE, PT_PHDR,
    /// PT_GNU_RELRO, etc. The raw `p_type` is preserved so a future
    /// dispatcher can branch on it without re-parsing.
    Other(u32),
}

/// One row of the parsed program-header table. Every numeric field is
/// the spec-typed width (u32 for flags, u64 for offset/vaddr/size/
/// align) — we don't downcast to `usize` here so a 32-bit host build
/// (running unit tests on aarch32) doesn't lose information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramHeader {
    pub kind: SegmentKind,
    pub flags: u32,
    pub offset: u64,
    pub vaddr: u64,
    pub paddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub align: u64,
}

impl ProgramHeader {
    /// Convenience: did the toolchain mark this segment readable?
    pub fn is_readable(&self) -> bool {
        self.flags & PF_R != 0
    }
    /// Convenience: writable segments need a writable page-table
    /// permission bit at load time.
    pub fn is_writable(&self) -> bool {
        self.flags & PF_W != 0
    }
    /// Convenience: executable segments map with PT_X-equivalent;
    /// non-executable PT_GNU_STACK records the negation of this for
    /// the stack region the loader synthesises.
    pub fn is_executable(&self) -> bool {
        self.flags & PF_X != 0
    }
}

/// The validated, parsed form of an ELF64 input. Owns the program-
/// header `Vec` (small — typically 8-12 entries even for fat dynamic
/// binaries) and the headline file-header fields the loader needs.
///
/// We do NOT retain a borrow of the input bytes. The loader will need
/// the bytes again at segment-mapping time (#472b) and the caller is
/// expected to keep the input slice alive for that follow-up call.
/// Keeping the bytes here would force a lifetime parameter on every
/// downstream type, which gets infectious quickly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedElf {
    /// `e_type`. Always one of ET_EXEC / ET_DYN — we reject every
    /// other value at parse time.
    pub elf_type: u16,
    /// `e_machine`. Always EM_X86_64 in this slice; preserved as a
    /// raw u16 so a future aarch64 widening is one validation tweak.
    pub machine: u16,
    /// `e_entry` — the virtual address of the first instruction. The
    /// loader trampolines into this once relocations + TLS init are
    /// done.
    pub entry: u64,
    /// `e_ident[EI_OSABI]` — preserved so the loader knows whether to
    /// emulate strict SysV or Linux-extension semantics.
    pub osabi: u8,
    /// All program-header entries, in file order. Order matters for
    /// PT_PHDR / PT_TLS placement during loading.
    pub program_headers: Vec<ProgramHeader>,
}

impl ParsedElf {
    /// Iterate just the PT_LOAD segments — the only ones that get
    /// page-mapped. Convenience for the loader's `for seg in elf.
    /// load_segments() { map(seg); }` loop in #472b.
    pub fn load_segments(&self) -> impl Iterator<Item = &ProgramHeader> {
        self.program_headers
            .iter()
            .filter(|ph| matches!(ph.kind, SegmentKind::Load))
    }

    /// Borrow the PT_INTERP entry if present. Static binaries have
    /// none; dynamically-linked binaries name `/lib64/ld-linux-x86-
    /// 64.so.2` (or similar) here. Useful for #472d to detect
    /// "needs dynamic linker" without re-walking the table.
    pub fn interp_segment(&self) -> Option<&ProgramHeader> {
        self.program_headers
            .iter()
            .find(|ph| matches!(ph.kind, SegmentKind::Interp))
    }
}

// -- Parser --------------------------------------------------------

/// Parse an ELF64 binary's file header + program-header table from a
/// byte slice. Returns a fully-validated `ParsedElf` on success or a
/// descriptive `ElfError` on any malformed input.
///
/// This is the only public entry point. Internal helpers (`read_u16_le`,
/// `read_u32_le`, `read_u64_le`) are private — they validate slice
/// length once, then read with `from_le_bytes` against the known-good
/// sub-slice.
///
/// No memory mapping happens here; no segment payload is dereferenced.
/// The loader (#472b) takes the resulting `ParsedElf` plus the original
/// slice and carves the address space as a separate operation.
pub fn parse(bytes: &[u8]) -> Result<ParsedElf, ElfError> {
    // Step 1: file is at least the 64-byte header. Without the ident
    // block we can't even check magic.
    if bytes.len() < ELF64_HEADER_SIZE {
        return Err(ElfError::Truncated);
    }

    // Step 2: validate the 16-byte ident block. Any failure here is
    // categorical — wrong file format, no point reading further.
    if bytes[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if bytes[4] != ELFCLASS64 {
        return Err(ElfError::WrongClass);
    }
    if bytes[5] != ELFDATA2LSB {
        return Err(ElfError::WrongEndian);
    }
    if bytes[6] != EV_CURRENT {
        return Err(ElfError::WrongIdentVersion);
    }
    let osabi = bytes[7];
    if osabi != ELFOSABI_SYSV && osabi != ELFOSABI_LINUX {
        return Err(ElfError::WrongAbi);
    }
    // bytes[8] is EI_ABIVERSION; spec says "0 unless something else is
    // explicitly defined for the OSABI" — Linux defines no special
    // version here, so any value is technically permissible. We don't
    // reject on it.
    // bytes[9..16] is EI_PAD — should be zero; spec says "currently
    // unused" but doesn't mandate the parser reject non-zero, so we
    // skip the check (matches goblin / readelf behaviour).

    // Step 3: read the rest of the file header. All multi-byte fields
    // are little-endian per ELFDATA2LSB.
    let e_type = read_u16_le(&bytes[16..18]);
    let e_machine = read_u16_le(&bytes[18..20]);
    // e_version (e_ident has its own EI_VERSION; this is the second
    // version field — same value, different field). We don't validate
    // it against EV_CURRENT here because real-world toolchains have
    // emitted EV_NONE in this slot historically while keeping
    // EI_VERSION at 1; readelf tolerates it.
    let _e_version = read_u32_le(&bytes[20..24]);
    let e_entry = read_u64_le(&bytes[24..32]);
    let e_phoff = read_u64_le(&bytes[32..40]);
    let _e_shoff = read_u64_le(&bytes[40..48]);
    let _e_flags = read_u32_le(&bytes[48..52]);
    let e_ehsize = read_u16_le(&bytes[52..54]);
    let e_phentsize = read_u16_le(&bytes[54..56]);
    let e_phnum = read_u16_le(&bytes[56..58]);
    // e_shentsize / e_shnum / e_shstrndx are at 58..64 — section
    // header metadata that the loader doesn't need. We deliberately
    // don't even read them; #472b loads from the program-header table.

    // Step 4: validate type / machine.
    if e_type != ET_EXEC && e_type != ET_DYN {
        return Err(ElfError::WrongType);
    }
    if e_machine != EM_X86_64 {
        return Err(ElfError::WrongMachine);
    }

    // Step 5: header-size sanity. If `e_ehsize` doesn't match the
    // spec's 64, the toolchain is using a non-standard ELF dialect we
    // shouldn't try to load.
    if e_ehsize as usize != ELF64_HEADER_SIZE {
        return Err(ElfError::BadHeaderSize);
    }
    if e_phentsize as usize != ELF64_PHENT_SIZE {
        return Err(ElfError::BadPhentSize);
    }

    // Step 6: bounds-check the program-header table. `e_phoff +
    // e_phnum * e_phentsize` must fit `bytes.len()` without wrapping.
    let phnum = e_phnum as usize;
    let phoff_usize = match usize_from_u64(e_phoff) {
        Some(v) => v,
        None => return Err(ElfError::PhdrTableOverflow),
    };
    let phtab_size = match phnum.checked_mul(ELF64_PHENT_SIZE) {
        Some(v) => v,
        None => return Err(ElfError::PhdrTableOverflow),
    };
    let phtab_end = match phoff_usize.checked_add(phtab_size) {
        Some(v) => v,
        None => return Err(ElfError::PhdrTableOverflow),
    };
    if phtab_end > bytes.len() {
        return Err(ElfError::PhdrTableOverflow);
    }

    // Step 7: walk the table.
    let mut program_headers = Vec::with_capacity(phnum);
    for i in 0..phnum {
        let off = phoff_usize + i * ELF64_PHENT_SIZE;
        let entry = &bytes[off..off + ELF64_PHENT_SIZE];
        // ELF64 program header layout (56 bytes):
        //   p_type   u32  @  0
        //   p_flags  u32  @  4
        //   p_offset u64  @  8
        //   p_vaddr  u64  @ 16
        //   p_paddr  u64  @ 24
        //   p_filesz u64  @ 32
        //   p_memsz  u64  @ 40
        //   p_align  u64  @ 48
        let p_type = read_u32_le(&entry[0..4]);
        let p_flags = read_u32_le(&entry[4..8]);
        let p_offset = read_u64_le(&entry[8..16]);
        let p_vaddr = read_u64_le(&entry[16..24]);
        let p_paddr = read_u64_le(&entry[24..32]);
        let p_filesz = read_u64_le(&entry[32..40]);
        let p_memsz = read_u64_le(&entry[40..48]);
        let p_align = read_u64_le(&entry[48..56]);

        // Bounds-check loadable segments against the input. PT_LOAD
        // is the one that the loader will read from at map time, so
        // a `p_offset + p_filesz` past `bytes.len()` is a hard error.
        // PT_INTERP names a string the loader will read out — same
        // bounds requirement. Other segment kinds (PT_GNU_STACK,
        // PT_TLS) can have zero filesz, which means "no in-file
        // payload"; the bounds check elides naturally.
        if matches!(p_type, PT_LOAD | PT_INTERP) && p_filesz > 0 {
            let off_usize = match usize_from_u64(p_offset) {
                Some(v) => v,
                None => return Err(ElfError::SegmentOutOfBounds),
            };
            let sz_usize = match usize_from_u64(p_filesz) {
                Some(v) => v,
                None => return Err(ElfError::SegmentOutOfBounds),
            };
            let end = match off_usize.checked_add(sz_usize) {
                Some(v) => v,
                None => return Err(ElfError::SegmentOutOfBounds),
            };
            if end > bytes.len() {
                return Err(ElfError::SegmentOutOfBounds);
            }
        }

        let kind = match p_type {
            PT_LOAD => SegmentKind::Load,
            PT_INTERP => SegmentKind::Interp,
            PT_GNU_STACK => SegmentKind::GnuStack,
            PT_TLS => SegmentKind::Tls,
            other => SegmentKind::Other(other),
        };

        program_headers.push(ProgramHeader {
            kind,
            flags: p_flags,
            offset: p_offset,
            vaddr: p_vaddr,
            paddr: p_paddr,
            filesz: p_filesz,
            memsz: p_memsz,
            align: p_align,
        });
    }

    Ok(ParsedElf {
        elf_type: e_type,
        machine: e_machine,
        entry: e_entry,
        osabi,
        program_headers,
    })
}

// -- Loader (#519, #520) -------------------------------------------
//
// `load_segments` is the second slice of the #472 epic — it consumes
// the parser's `ParsedElf` plus the original ELF blob and produces an
// in-memory `AddressSpace` ready for the trampoline (#521) to install
// into a real page table.
//
// Composition shape mirrors the parser: returns a typed `Result<...,
// LoadOrParseError>` so a single call site can branch on parser-side
// vs. loader-side failures without string-matching. The error enum
// flattens `ElfError` and `LoaderError` into one — neither is `Display`
// today, but both are `Copy + PartialEq` so callers stash them, log
// them, and react.
//
// PT_INTERP detection (#520, folded into this commit per the task
// description) lives in this same function: the moment the parser
// hands us a `SegmentKind::Interp`, we error out with
// `LoaderError::DynamicLoaderRequired` rather than try to chase the
// interpreter path. Tier-1 only hosts statically-linked binaries; the
// musl-based dynamic loader path lands in #522 (WWWW's parallel
// vendor/musl/ track).

/// Errors `load_segments` can return. Wraps both parser-side
/// `ElfError` (for blob structural issues) and loader-side
/// `LoaderError` (for semantic / resource issues) so a single call
/// site can branch by variant. Stays `Copy` for the same reason as
/// its components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOrParseError {
    /// The parser rejected the input — see `ElfError` variants.
    Parse(ElfError),
    /// The parser succeeded but the loader rejected a per-segment
    /// invariant (overlap, BSS shape, W^X, OOM, dynamic-linker).
    Load(LoaderError),
}

impl From<ElfError> for LoadOrParseError {
    fn from(e: ElfError) -> Self {
        LoadOrParseError::Parse(e)
    }
}

impl From<LoaderError> for LoadOrParseError {
    fn from(e: LoaderError) -> Self {
        LoadOrParseError::Load(e)
    }
}

/// Load every PT_LOAD segment from `parsed` into a fresh `AddressSpace`,
/// using `bytes` as the source of segment payloads.
///
/// The function:
///   1. Refuses dynamically-linked binaries up front (PT_INTERP
///      present → `LoaderError::DynamicLoaderRequired`, #520).
///   2. Iterates PT_LOAD entries in the parser-yielded order.
///   3. For each entry: derives the permission from `p_flags`,
///      slices the file content from `bytes[p_offset..p_offset+p_filesz]`,
///      hands them to `AddressSpace::push_segment` which page-aligns
///      the allocation, copies the file content, and zeros the BSS.
///   4. Detects overlap on push (push_segment's invariant); the
///      first overlap returns `LoaderError::OverlappingSegments`
///      and the partial AddressSpace is dropped (every previously-
///      pushed segment's allocation is reclaimed via `Drop`).
///
/// `bytes` is the same `&[u8]` the caller fed into `parse(...)` —
/// the parser already validated `[p_offset, p_offset + p_filesz)`
/// fits the slice for every PT_LOAD (elf.rs:424), so the slicing
/// here is bounds-safe by construction. We re-check the length
/// defensively — a future refactor could change the parser's
/// validation surface without touching this site.
pub fn load_segments(
    parsed: &ParsedElf,
    bytes: &[u8],
) -> Result<AddressSpace, LoadOrParseError> {
    // #520: PT_INTERP detection. Static binaries have no PT_INTERP;
    // dynamically-linked ones name the loader (typically
    // `/lib64/ld-linux-x86-64.so.2`). The musl-based loader (#522)
    // will replace this branch with "thread the interpreter through
    // a recursive load_segments call against the loader's own ELF".
    if parsed.interp_segment().is_some() {
        return Err(LoaderError::DynamicLoaderRequired.into());
    }

    let mut address_space = AddressSpace::new(parsed.entry);

    for ph in parsed.load_segments() {
        // Derive the permission shape. W^X violations and bare PF_X
        // are rejected before we allocate — no point carving pages
        // we'll just drop.
        let perm = perm_from_p_flags(ph.flags)?;

        // Slice the file content out of the ELF blob. The parser
        // already bounds-checked this for PT_LOAD entries with
        // non-zero filesz (elf.rs:424). For zero-filesz segments
        // (.bss-only sections), `file_content` is an empty slice and
        // `push_segment` zeros the entire `mem_size`.
        let file_content: &[u8] = if ph.filesz == 0 {
            &[]
        } else {
            // Defensive bounds re-check — see function-level comment.
            let off = ph.offset as usize;
            let sz = ph.filesz as usize;
            let end = off
                .checked_add(sz)
                .ok_or(LoadOrParseError::Parse(ElfError::SegmentOutOfBounds))?;
            if end > bytes.len() {
                return Err(LoadOrParseError::Parse(ElfError::SegmentOutOfBounds));
            }
            &bytes[off..end]
        };

        // mem_size as usize. ELF stores it as u64; on 64-bit hosts
        // (every UEFI target) the cast is loss-free, but a 32-bit
        // unit-test host needs the explicit conversion to detect a
        // segment that wouldn't fit the address space anyway.
        let mem_size = match usize_from_u64(ph.memsz) {
            Some(v) => v,
            None => {
                return Err(LoaderError::AddressSpaceTooLarge.into());
            }
        };

        address_space.push_segment(ph.vaddr, mem_size, perm, file_content)?;
    }

    Ok(address_space)
}

// -- Internal helpers ----------------------------------------------
//
// All four take a slice that the caller has *already* bounded (every
// caller above slices `bytes[a..b]` first), so the `[..2]` / `[..4]` /
// `[..8]` patterns here are infallible by construction. We use
// `try_into().unwrap()` on the array-conversion to make the panic
// path explicit (it's a programming error, not malformed input — if
// it ever fires the bug is in the caller's slice arithmetic).

fn read_u16_le(s: &[u8]) -> u16 {
    let arr: [u8; 2] = s[..2].try_into().expect("read_u16_le called with <2 bytes");
    u16::from_le_bytes(arr)
}

fn read_u32_le(s: &[u8]) -> u32 {
    let arr: [u8; 4] = s[..4].try_into().expect("read_u32_le called with <4 bytes");
    u32::from_le_bytes(arr)
}

fn read_u64_le(s: &[u8]) -> u64 {
    let arr: [u8; 8] = s[..8].try_into().expect("read_u64_le called with <8 bytes");
    u64::from_le_bytes(arr)
}

/// Cast a `u64` (ELF on-disk field width) to `usize` (Rust's index
/// type) without panic. Returns `None` if `v` exceeds `usize::MAX` —
/// only possible on a 32-bit host running these tests, since on UEFI
/// the kernel target is x86_64 (or aarch64) where `usize` is 64 bits.
/// The `as usize` cast that everyone reaches for would silently
/// truncate; this helper makes the failure explicit.
fn usize_from_u64(v: u64) -> Option<usize> {
    if v > usize::MAX as u64 {
        None
    } else {
        Some(v as usize)
    }
}

// -- Tests ---------------------------------------------------------
//
// Convention matches AAAA's #460 linuxkpi modules — `#[cfg(test)] mod
// tests` block at the bottom with one `#[test]` per assertion. The
// kernel's bin target carries `test = false` (Cargo.toml:112) so these
// don't run under the default `cargo test` invocation today — they
// type-check, and a future host-target test harness slice can flip the
// switch without touching this module.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::test_fixtures::TINY_ELF;

    /// Happy path: TINY_ELF parses cleanly, headline fields match the
    /// hand-built bytes, both program headers are recognised.
    #[test]
    fn parses_tiny_elf_fixture() {
        let elf = parse(TINY_ELF).expect("TINY_ELF must parse");
        assert_eq!(elf.elf_type, ET_EXEC);
        assert_eq!(elf.machine, EM_X86_64);
        assert_eq!(elf.entry, 0x0040_1000);
        assert_eq!(elf.osabi, ELFOSABI_SYSV);
        assert_eq!(elf.program_headers.len(), 2);
    }

    /// PT_LOAD segment is correctly classified and the flag /
    /// offset / size fields round-trip.
    #[test]
    fn pt_load_classified_and_fields_round_trip() {
        let elf = parse(TINY_ELF).expect("TINY_ELF must parse");
        let load = elf.load_segments().next().expect("expected PT_LOAD");
        assert_eq!(load.kind, SegmentKind::Load);
        assert_eq!(load.flags, PF_R | PF_X);
        assert_eq!(load.offset, 0x100);
        assert_eq!(load.vaddr, 0x0040_1000);
        assert_eq!(load.filesz, 0x10);
        assert_eq!(load.memsz, 0x10);
        assert_eq!(load.align, 0x1000);
        assert!(load.is_readable());
        assert!(load.is_executable());
        assert!(!load.is_writable());
    }

    /// PT_GNU_STACK is correctly classified as `GnuStack`, NOT
    /// `Other` — proves the GNU-extended `p_type` branch fires.
    #[test]
    fn pt_gnu_stack_classified() {
        let elf = parse(TINY_ELF).expect("TINY_ELF must parse");
        let stack = elf
            .program_headers
            .iter()
            .find(|p| matches!(p.kind, SegmentKind::GnuStack))
            .expect("expected PT_GNU_STACK");
        assert!(stack.is_readable());
        assert!(stack.is_writable());
        assert!(!stack.is_executable());
    }

    /// `interp_segment()` returns None on a static binary — TINY_ELF
    /// has no PT_INTERP entry.
    #[test]
    fn interp_segment_absent_on_static_binary() {
        let elf = parse(TINY_ELF).expect("TINY_ELF must parse");
        assert!(elf.interp_segment().is_none());
    }

    /// Empty input is `Truncated`, not a panic.
    #[test]
    fn empty_input_is_truncated() {
        assert_eq!(parse(&[]), Err(ElfError::Truncated));
    }

    /// 63-byte input is `Truncated` (one byte short of header).
    #[test]
    fn one_byte_short_of_header_is_truncated() {
        let buf = [0u8; 63];
        assert_eq!(parse(&buf), Err(ElfError::Truncated));
    }

    /// Non-ELF input (like a PE binary's MZ header) is `BadMagic`.
    #[test]
    fn bad_magic_rejected() {
        let mut buf = [0u8; 64];
        buf[0..2].copy_from_slice(b"MZ");
        assert_eq!(parse(&buf), Err(ElfError::BadMagic));
    }

    /// 32-bit ELF (ELFCLASS32) is `WrongClass`.
    #[test]
    fn elfclass32_rejected() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[4] = 1; // ELFCLASS32
        assert_eq!(parse(&buf), Err(ElfError::WrongClass));
    }

    /// Big-endian ELF (ELFDATA2MSB) is `WrongEndian`.
    #[test]
    fn elfdata2msb_rejected() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[5] = 2; // ELFDATA2MSB
        assert_eq!(parse(&buf), Err(ElfError::WrongEndian));
    }

    /// FreeBSD ABI (9) is `WrongAbi`.
    #[test]
    fn freebsd_abi_rejected() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[7] = 9;
        assert_eq!(parse(&buf), Err(ElfError::WrongAbi));
    }

    /// ELFOSABI_LINUX (3) is *accepted* (alongside SYSV) — Linux
    /// extension toolchains emit this, and we host them just fine.
    #[test]
    fn elfosabi_linux_accepted() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[7] = ELFOSABI_LINUX;
        let elf = parse(&buf).expect("ELFOSABI_LINUX must parse");
        assert_eq!(elf.osabi, ELFOSABI_LINUX);
    }

    /// ET_REL (relocatable object file, type=1) is `WrongType` —
    /// we only host executables (ET_EXEC) and PIE (ET_DYN).
    #[test]
    fn et_rel_rejected() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[16] = 1; // ET_REL
        buf[17] = 0;
        assert_eq!(parse(&buf), Err(ElfError::WrongType));
    }

    /// ET_DYN (PIE) is *accepted* — same parse path, downstream
    /// loader applies relocations.
    #[test]
    fn et_dyn_accepted() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[16] = ET_DYN as u8;
        buf[17] = 0;
        let elf = parse(&buf).expect("ET_DYN must parse");
        assert_eq!(elf.elf_type, ET_DYN);
    }

    /// EM_AARCH64 (183) is `WrongMachine` — aarch64 process support
    /// is a separate epic.
    #[test]
    fn em_aarch64_rejected() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[18] = 183; // EM_AARCH64
        buf[19] = 0;
        assert_eq!(parse(&buf), Err(ElfError::WrongMachine));
    }

    /// Truncated program-header table (e_phnum claims 2 entries but
    /// the file only has room for 1) is `PhdrTableOverflow`.
    #[test]
    fn truncated_phdr_table_rejected() {
        // Take the 64-byte header + 56-byte single phdr = 120 bytes
        // BUT keep e_phnum = 2. Parser should refuse to read past
        // the end.
        let mut buf = TINY_ELF[..64 + ELF64_PHENT_SIZE].to_vec();
        // e_phnum lives at offset 56 (after e_shentsize at 54).
        // It already says 2 in TINY_ELF — leave it. The shortened
        // buffer can't hold the 2nd entry → expect overflow.
        assert_eq!(parse(&buf), Err(ElfError::PhdrTableOverflow));
        // Sanity: bumping it to 1 makes the same buffer parse.
        buf[56] = 1;
        buf[57] = 0;
        let elf = parse(&buf).expect("1-phdr fixture must parse");
        assert_eq!(elf.program_headers.len(), 1);
    }

    /// PT_LOAD whose `p_offset + p_filesz` exceeds `bytes.len()`
    /// is `SegmentOutOfBounds`.
    #[test]
    fn pt_load_out_of_bounds_rejected() {
        // Truncate after the program-header table (no payload at
        // p_offset = 0x100 anymore). The fixture's PT_LOAD claims
        // 0x10 bytes at p_offset 0x100; an input of length 0xC0 has
        // no room for them.
        let buf = &TINY_ELF[..0xC0];
        assert_eq!(parse(buf), Err(ElfError::SegmentOutOfBounds));
    }

    /// `e_ehsize` other than 64 is `BadHeaderSize`.
    #[test]
    fn bad_e_ehsize_rejected() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[52] = 0x80; // e_ehsize = 128
        assert_eq!(parse(&buf), Err(ElfError::BadHeaderSize));
    }

    /// `e_phentsize` other than 56 is `BadPhentSize`.
    #[test]
    fn bad_e_phentsize_rejected() {
        let mut buf: [u8; 256] = [0; 256];
        buf[..TINY_ELF.len()].copy_from_slice(TINY_ELF);
        buf[54] = 0x40; // e_phentsize = 64
        assert_eq!(parse(&buf), Err(ElfError::BadPhentSize));
    }

    // -- Loader tests (#519 + #520) --------------------------------
    //
    // The loader builds on the parser — every test here parses
    // `LOADER_ELF` / `INTERP_ELF` / `OVERLAP_ELF` first, then calls
    // `load_segments` against the parsed result. The fixtures live
    // in `test_fixtures.rs`; the parser tests above use the same
    // module so the fixtures stay reusable.

    use crate::process::address_space::SegmentPerm;
    use crate::process::test_fixtures::{INTERP_ELF, LOADER_ELF, OVERLAP_ELF};

    /// LOADER_ELF loads cleanly: two PT_LOAD segments materialise
    /// in the address space, in the order the parser yielded them.
    #[test]
    fn load_segments_loader_elf_two_segments() {
        let elf = parse(LOADER_ELF).expect("LOADER_ELF must parse");
        let address_space = load_segments(&elf, LOADER_ELF).expect("load must succeed");
        assert_eq!(address_space.entry_point, 0x0040_1000);
        assert_eq!(address_space.segments.len(), 2);
        assert_eq!(address_space.segments[0].vaddr, 0x0040_1000);
        assert_eq!(address_space.segments[0].mem_size, 0x10);
        assert_eq!(address_space.segments[0].file_size, 0x10);
        assert_eq!(address_space.segments[0].perm, SegmentPerm::ReadExecute);
        assert_eq!(address_space.segments[1].vaddr, 0x0040_2000);
        assert_eq!(address_space.segments[1].mem_size, 0x20);
        assert_eq!(address_space.segments[1].file_size, 0x08);
        assert_eq!(address_space.segments[1].perm, SegmentPerm::ReadWrite);
    }

    /// File content lands at the head of the segment's allocation —
    /// distinct payload bytes (0xAA / 0xBB) round-trip from the ELF
    /// blob through the loader's copy.
    #[test]
    fn load_segments_copies_file_content() {
        let elf = parse(LOADER_ELF).expect("LOADER_ELF must parse");
        let address_space = load_segments(&elf, LOADER_ELF).expect("load must succeed");

        // .text segment: 16 bytes of 0xAA at the head, no BSS.
        let text = address_space.segments[0].pages_view();
        assert_eq!(&text[..0x10], &[0xaa; 0x10]);

        // .data segment: 8 bytes of 0xBB at the head, then 24 bytes
        // of zero (BSS). The fixture's payload byte is deliberately
        // 0xBB (non-zero) so a successful copy is distinguishable
        // from leftover BSS-zero.
        let data = address_space.segments[1].pages_view();
        assert_eq!(&data[..0x08], &[0xbb; 0x08]);
    }

    /// BSS region (mem_size - file_size bytes after the file content)
    /// is zeroed. This is the most-likely-to-regress invariant of the
    /// loader: if the page allocation isn't zero-init'd and the loader
    /// "forgets" to zero the BSS, a static binary's `.bss` would carry
    /// whatever bytes happened to be on the heap previously.
    #[test]
    fn load_segments_zeros_bss() {
        let elf = parse(LOADER_ELF).expect("LOADER_ELF must parse");
        let address_space = load_segments(&elf, LOADER_ELF).expect("load must succeed");

        // .data segment's BSS = mem_size 0x20 - file_size 0x08 = 0x18 bytes.
        let data = address_space.segments[1].pages_view();
        for (i, byte) in data[0x08..0x20].iter().enumerate() {
            assert_eq!(
                *byte, 0,
                "BSS byte {} (offset 0x{:x}) is 0x{:02x}, expected zero",
                i,
                0x08 + i,
                byte
            );
        }
    }

    /// PT_INTERP-bearing binary is rejected with
    /// `LoaderError::DynamicLoaderRequired` (#520). The fixture's
    /// PT_INTERP entry names `/lib64/ld-linux-x86-64.so.2`; static
    /// binaries have no PT_INTERP. Tier-1 doesn't yet host a
    /// dynamic loader.
    #[test]
    fn load_segments_rejects_pt_interp() {
        let elf = parse(INTERP_ELF).expect("INTERP_ELF must parse");
        let err = load_segments(&elf, INTERP_ELF).unwrap_err();
        assert_eq!(err, LoadOrParseError::Load(LoaderError::DynamicLoaderRequired));
    }

    /// Overlapping PT_LOAD segments are rejected with
    /// `LoaderError::OverlappingSegments`. OVERLAP_ELF places the
    /// .data segment at vaddr 0x0040_1008 — inside the .text
    /// segment's [0x0040_1000, 0x0040_1010) range.
    #[test]
    fn load_segments_rejects_overlapping_segments() {
        let elf = parse(OVERLAP_ELF).expect("OVERLAP_ELF must parse");
        let err = load_segments(&elf, OVERLAP_ELF).unwrap_err();
        assert_eq!(err, LoadOrParseError::Load(LoaderError::OverlappingSegments));
    }

    /// TINY_ELF (parser fixture) loads fine — its PT_LOAD payload
    /// is 16 bytes of zero at offset 0x100. This proves the loader
    /// works against the parser's existing fixture too, not just
    /// the new loader-specific ones.
    #[test]
    fn load_segments_tiny_elf_works() {
        let elf = parse(TINY_ELF).expect("TINY_ELF must parse");
        let address_space = load_segments(&elf, TINY_ELF).expect("load must succeed");
        assert_eq!(address_space.entry_point, 0x0040_1000);
        assert_eq!(address_space.segments.len(), 1);
        assert_eq!(address_space.segments[0].vaddr, 0x0040_1000);
        assert_eq!(address_space.segments[0].perm, SegmentPerm::ReadExecute);
    }

    /// `LoadOrParseError::Parse` flows through when the upstream
    /// parser failed. We can't readily produce a "parsed but the
    /// underlying bytes got truncated" case without rebuilding the
    /// fixture; this test just confirms the `From<ElfError>` impl
    /// composes a `LoadOrParseError::Parse(...)` variant correctly.
    #[test]
    fn load_or_parse_error_from_elf_error() {
        let err: LoadOrParseError = ElfError::BadMagic.into();
        assert_eq!(err, LoadOrParseError::Parse(ElfError::BadMagic));
    }

    /// `LoadOrParseError::Load` flows through from any LoaderError
    /// — same shape test as the ElfError one above, for symmetry.
    #[test]
    fn load_or_parse_error_from_loader_error() {
        let err: LoadOrParseError = LoaderError::OverlappingSegments.into();
        assert_eq!(err, LoadOrParseError::Load(LoaderError::OverlappingSegments));
    }
}
