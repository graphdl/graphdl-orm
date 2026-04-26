// crates/arest-kernel/src/process/test_fixtures.rs
//
// Hand-crafted ELF64 byte fixtures for the parser unit tests. Kept
// small (headers only — no actual code segments) because #518's scope
// is *parse* only, not load-and-execute. The bytes here are valid
// according to the ELF64 spec (System V Application Binary Interface,
// AMD64 Architecture Processor Supplement) so the parser produces a
// fully-populated `ParsedElf` from them, but they are NOT a runnable
// program — every PT_LOAD `p_offset` points at zeroes inside the
// fixture rather than executable instructions.
//
// Layout of TINY_ELF
// ------------------
// File header       : bytes 0x000..0x040  (64 bytes — ELF64 header)
// Program header 0  : bytes 0x040..0x078  (56 bytes — PT_LOAD .text)
// Program header 1  : bytes 0x078..0x0B0  (56 bytes — PT_GNU_STACK)
// Padding to 0x100  : zero-filled tail so PT_LOAD `p_offset` = 0x100
//                     resolves inside the slice (parser only validates
//                     it's in-range; doesn't dereference it).
//
// Why two program headers
// -----------------------
// The smallest interesting case for the parser exercises both the
// PT_LOAD branch (the workhorse — every static binary needs at least
// one) and one of the PT_GNU_STACK / PT_INTERP / PT_TLS branches so
// the segment-kind dispatch isn't dead code in the test path. PT_GNU_-
// STACK is the lightest of the three (no payload semantics — just a
// permissions hint) and a real `gcc -static` binary always emits one,
// so it's the most representative second header.
//
// Why we don't `include_bytes!` a real /bin/true
// ----------------------------------------------
// Two reasons. First, vendoring a real Linux ELF would extend the
// kernel's license footprint — even a tiny coreutils binary inherits
// GPL-3.0-or-later, which is incompatible with the kernel's plain
// AGPL-3.0-or-later default. Second, the parser needs to be exercised
// against deliberately *malformed* inputs (bad magic, wrong class,
// truncated tables); building those mutants from a real binary is
// awkward, but trivial when the bytes are already a literal table we
// can `&[..]` into.

#![allow(dead_code)]

/// Minimal valid static ELF64 fixture. 256 bytes total: 64-byte file
/// header + 2 × 56-byte program headers + 80 bytes of zero pad so
/// PT_LOAD's `p_offset = 0x100` is in-range (the parser doesn't read
/// segment payload, just validates the offset+size fit the slice when
/// asked). Hand-crafted little-endian per AMD64 SysV ABI.
///
/// File header fields:
///   e_ident: \x7fELF, class=64, data=LE, version=1, abi=SYSV/Linux=0,
///            abi_version=0, padding=0
///   e_type     = ET_EXEC (2)
///   e_machine  = EM_X86_64 (62 = 0x3e)
///   e_version  = 1
///   e_entry    = 0x0040_1000  (text segment vaddr + 0)
///   e_phoff    = 0x40         (program headers start right after file header)
///   e_shoff    = 0            (no section headers — we don't need them)
///   e_flags    = 0
///   e_ehsize   = 64           (ELF64 header size)
///   e_phentsize= 56           (ELF64 program-header entry size)
///   e_phnum    = 2
///   e_shentsize= 0
///   e_shnum    = 0
///   e_shstrndx = 0
///
/// Program header 0 (PT_LOAD .text):
///   p_type   = 1 (PT_LOAD)
///   p_flags  = 5 (PF_R | PF_X)
///   p_offset = 0x100
///   p_vaddr  = 0x0040_1000
///   p_paddr  = 0x0040_1000
///   p_filesz = 0x10
///   p_memsz  = 0x10
///   p_align  = 0x1000
///
/// Program header 1 (PT_GNU_STACK):
///   p_type   = 0x6474_e551 (PT_GNU_STACK)
///   p_flags  = 6 (PF_R | PF_W — non-executable stack)
///   p_offset = 0
///   p_vaddr  = 0
///   p_paddr  = 0
///   p_filesz = 0
///   p_memsz  = 0
///   p_align  = 0x10
pub const TINY_ELF: &[u8] = &[
    // -------- ELF64 file header (offsets 0x00..0x40) --------
    // e_ident[EI_MAG0..EI_MAG3] = \x7fELF
    0x7f, 0x45, 0x4c, 0x46,
    // e_ident[EI_CLASS] = 2 (ELFCLASS64)
    0x02,
    // e_ident[EI_DATA] = 1 (ELFDATA2LSB — little-endian)
    0x01,
    // e_ident[EI_VERSION] = 1 (EV_CURRENT)
    0x01,
    // e_ident[EI_OSABI] = 0 (ELFOSABI_SYSV — also accepted as Linux)
    0x00,
    // e_ident[EI_ABIVERSION] = 0
    0x00,
    // e_ident[EI_PAD..EI_NIDENT] = 0,0,0,0,0,0,0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_type = ET_EXEC (2)
    0x02, 0x00,
    // e_machine = EM_X86_64 (62 = 0x3e)
    0x3e, 0x00,
    // e_version = 1
    0x01, 0x00, 0x00, 0x00,
    // e_entry = 0x0000_0000_0040_1000
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_phoff = 0x40
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_shoff = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_flags = 0
    0x00, 0x00, 0x00, 0x00,
    // e_ehsize = 64
    0x40, 0x00,
    // e_phentsize = 56
    0x38, 0x00,
    // e_phnum = 2
    0x02, 0x00,
    // e_shentsize = 0
    0x00, 0x00,
    // e_shnum = 0
    0x00, 0x00,
    // e_shstrndx = 0
    0x00, 0x00,

    // -------- Program header 0: PT_LOAD .text (offsets 0x40..0x78) --------
    // p_type = PT_LOAD (1)
    0x01, 0x00, 0x00, 0x00,
    // p_flags = PF_R | PF_X (5)
    0x05, 0x00, 0x00, 0x00,
    // p_offset = 0x100
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr = 0x0000_0000_0040_1000
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_paddr = 0x0000_0000_0040_1000
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_filesz = 0x10
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_memsz = 0x10
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 0x1000
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Program header 1: PT_GNU_STACK (offsets 0x78..0xB0) --------
    // p_type = PT_GNU_STACK (0x6474_e551)
    0x51, 0xe5, 0x74, 0x64,
    // p_flags = PF_R | PF_W (6 — non-executable stack)
    0x06, 0x00, 0x00, 0x00,
    // p_offset = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_paddr = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_filesz = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_memsz = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 0x10
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Pad to 0x100 so PT_LOAD's p_offset=0x100 stays in-slice --------
    // 0xB0..0x100 is 80 bytes of zero.
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // -------- PT_LOAD payload (16 bytes of zeroes — not exercised) --------
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

// ---------------------------------------------------------------------------
// LOADER fixtures (#519)
// ---------------------------------------------------------------------------
//
// `LOADER_ELF` extends TINY_ELF's shape with a SECOND PT_LOAD whose
// `mem_size` exceeds its `file_size` — exercises the BSS-zeroing
// path. The two segments are placed at distinct virtual addresses so
// the loader's overlap check passes.
//
// Layout of LOADER_ELF (320 bytes total, padded to 0x140 for the
// second PT_LOAD payload to land in-slice):
//   File header       : 0x000..0x040  (64 bytes — same shape as TINY_ELF)
//   Program header 0  : 0x040..0x078  (PT_LOAD .text  — 16 file, 16 mem)
//   Program header 1  : 0x078..0x0B0  (PT_LOAD .data  — 8 file, 32 mem)
//   Program header 2  : 0x0B0..0x0E8  (PT_GNU_STACK)
//   Pad to 0x100      : zero-fill (PT_LOAD #0 payload)
//   PT_LOAD #0 payload: 0x100..0x110  (16 bytes — distinct pattern 0xAA)
//   PT_LOAD #1 payload: 0x110..0x118  (8 bytes  — distinct pattern 0xBB)
//   Pad to 0x140      : zero-fill (overshoot for slice safety)
//
// Distinct payload bytes (0xAA / 0xBB) are deliberately not zero so
// the loader's "did the file content actually copy?" assertion can
// distinguish a successful copy from leftover BSS-zero.
//
// Why a SECOND fixture instead of extending TINY_ELF
// --------------------------------------------------
// TINY_ELF is shared with the parser test suite (elf.rs:521+) and
// many of those tests assert `program_headers.len() == 2` — adding a
// third entry would break them. A new constant keeps the parser-
// surface fixture stable while the loader-surface fixture exercises
// the additional invariants. Same shape decision DDDD made when
// `readings/ui/components.md` grew its second fixture set.
pub const LOADER_ELF: &[u8] = &[
    // -------- ELF64 file header (offsets 0x00..0x40) --------
    // e_ident[EI_MAG0..EI_MAG3] = \x7fELF
    0x7f, 0x45, 0x4c, 0x46,
    // e_ident[EI_CLASS] = 2 (ELFCLASS64)
    0x02,
    // e_ident[EI_DATA] = 1 (ELFDATA2LSB)
    0x01,
    // e_ident[EI_VERSION] = 1 (EV_CURRENT)
    0x01,
    // e_ident[EI_OSABI] = 0 (ELFOSABI_SYSV)
    0x00,
    // e_ident[EI_ABIVERSION] + EI_PAD..EI_NIDENT = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_type = ET_EXEC (2)
    0x02, 0x00,
    // e_machine = EM_X86_64 (62 = 0x3e)
    0x3e, 0x00,
    // e_version = 1
    0x01, 0x00, 0x00, 0x00,
    // e_entry = 0x0000_0000_0040_1000
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_phoff = 0x40
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_shoff = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_flags = 0
    0x00, 0x00, 0x00, 0x00,
    // e_ehsize = 64
    0x40, 0x00,
    // e_phentsize = 56
    0x38, 0x00,
    // e_phnum = 3
    0x03, 0x00,
    // e_shentsize / e_shnum / e_shstrndx = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Program header 0: PT_LOAD .text (offsets 0x40..0x78) --------
    // p_type = PT_LOAD (1)
    0x01, 0x00, 0x00, 0x00,
    // p_flags = PF_R | PF_X (5)
    0x05, 0x00, 0x00, 0x00,
    // p_offset = 0x100
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr = 0x0000_0000_0040_1000
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_paddr = 0x0000_0000_0040_1000
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_filesz = 0x10
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_memsz = 0x10  (= filesz, no BSS for .text)
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 0x1000
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Program header 1: PT_LOAD .data+.bss (offsets 0x78..0xB0) --------
    // p_type = PT_LOAD (1)
    0x01, 0x00, 0x00, 0x00,
    // p_flags = PF_R | PF_W (6)
    0x06, 0x00, 0x00, 0x00,
    // p_offset = 0x110
    0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr = 0x0000_0000_0040_2000  (distinct from .text — no overlap)
    0x00, 0x20, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_paddr = 0x0000_0000_0040_2000
    0x00, 0x20, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_filesz = 0x08  (8 bytes of .data)
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_memsz = 0x20   (32 bytes total — BSS = mem - file = 24 bytes)
    0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 0x1000
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Program header 2: PT_GNU_STACK (offsets 0xB0..0xE8) --------
    // p_type = PT_GNU_STACK
    0x51, 0xe5, 0x74, 0x64,
    // p_flags = PF_R | PF_W
    0x06, 0x00, 0x00, 0x00,
    // p_offset / p_vaddr / p_paddr / p_filesz / p_memsz = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 0x10
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Pad 0xE8..0x100 (24 bytes of zero) --------
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,

    // -------- PT_LOAD #0 payload (16 bytes of 0xAA) — 0x100..0x110 --------
    0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
    0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,

    // -------- PT_LOAD #1 payload (8 bytes of 0xBB) — 0x110..0x118 --------
    0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb,

    // -------- Pad 0x118..0x140 (40 bytes of zero — slice tail) --------
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
];

/// Same shape as LOADER_ELF but with a PT_INTERP entry replacing the
/// PT_GNU_STACK third header. Used to exercise the dynamic-linker
/// rejection path (#520). The interpreter-name string at p_offset
/// 0x118 is `/lib64/ld-linux-x86-64.so.2\0` (28 bytes including the
/// trailing NUL); the loader rejects on the PT_INTERP presence alone
/// and never reads the string bytes, but we include them so a future
/// #522 reuse of this fixture sees a realistic blob and so the
/// parser's bounds check on PT_INTERP `p_filesz` (elf.rs:424) finds
/// the bytes in-range.
pub const INTERP_ELF: &[u8] = &[
    // ELF64 file header — same shape as LOADER_ELF.
    // Bytes 0..4: magic.
    0x7f, 0x45, 0x4c, 0x46,
    // Bytes 4..16: ident (class=64, data=LE, version=1, abi=SYSV, abi_ver=0, pad).
    0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 16..24: e_type=ET_EXEC, e_machine=EM_X86_64, e_version=1.
    0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00,
    // Bytes 24..32: e_entry = 0x0040_1000.
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 32..40: e_phoff = 0x40.
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 40..48: e_shoff = 0.
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 48..52: e_flags = 0.
    0x00, 0x00, 0x00, 0x00,
    // Bytes 52..54: e_ehsize = 64.
    0x40, 0x00,
    // Bytes 54..56: e_phentsize = 56.
    0x38, 0x00,
    // Bytes 56..58: e_phnum = 3.
    0x03, 0x00,
    // Bytes 58..60: e_shentsize = 0.
    0x00, 0x00,
    // Bytes 60..62: e_shnum = 0.
    0x00, 0x00,
    // Bytes 62..64: e_shstrndx = 0.
    0x00, 0x00,

    // PH 0: PT_LOAD .text (same as LOADER_ELF).
    0x01, 0x00, 0x00, 0x00,
    0x05, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // PH 1: PT_LOAD .data (same as LOADER_ELF).
    0x01, 0x00, 0x00, 0x00,
    0x06, 0x00, 0x00, 0x00,
    0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x20, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x20, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // PH 2: PT_INTERP — `/lib64/ld-linux-x86-64.so.2` at offset 0x118.
    // p_type = PT_INTERP (3)
    0x03, 0x00, 0x00, 0x00,
    // p_flags = PF_R (4)
    0x04, 0x00, 0x00, 0x00,
    // p_offset = 0x118
    0x18, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_paddr = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_filesz = 0x1c  (28 bytes — including trailing NUL)
    0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_memsz = 0x1c
    0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 1 (string, not memory-mappable)
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // Pad 0xE8..0x100 (24 bytes of zero).
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,

    // PT_LOAD #0 payload (16 bytes of 0xAA) — 0x100..0x110.
    0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
    0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,

    // PT_LOAD #1 payload (8 bytes of 0xBB) — 0x110..0x118.
    0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb,

    // PT_INTERP string `/lib64/ld-linux-x86-64.so.2\0` at 0x118
    // (28 bytes including trailing NUL — never read by the loader,
    // present for #522 reuse + parser bounds check satisfaction).
    b'/', b'l', b'i', b'b', b'6', b'4', b'/', b'l',
    b'd', b'-', b'l', b'i', b'n', b'u', b'x', b'-',
    b'x', b'8', b'6', b'-', b'6', b'4', b'.', b's',
    b'o', b'.', b'2', 0x00,
    // Pad 0x134..0x140 (12 bytes — buffer tail).
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Two-PT_LOAD fixture whose virtual ranges OVERLAP — the second
/// segment's `vaddr` lands inside the first's `[vaddr, vaddr+memsz)`.
/// Loader must reject on the second push with
/// `LoaderError::OverlappingSegments`.
///
/// Same byte layout as LOADER_ELF; only the second PT_LOAD's
/// `p_vaddr` is changed from `0x0040_2000` to `0x0040_1008` so it
/// overlaps the .text segment's tail.
pub const OVERLAP_ELF: &[u8] = &[
    // ELF64 file header — same shape as LOADER_ELF (e_phnum=2 here
    // since OVERLAP_ELF has only the two PT_LOAD entries).
    // Bytes 0..4: magic.
    0x7f, 0x45, 0x4c, 0x46,
    // Bytes 4..16: ident.
    0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 16..24: e_type=ET_EXEC, e_machine=EM_X86_64, e_version=1.
    0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00,
    // Bytes 24..32: e_entry = 0x0040_1000.
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 32..40: e_phoff = 0x40.
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 40..48: e_shoff = 0.
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // Bytes 48..52: e_flags = 0.
    0x00, 0x00, 0x00, 0x00,
    // Bytes 52..54: e_ehsize = 64.
    0x40, 0x00,
    // Bytes 54..56: e_phentsize = 56.
    0x38, 0x00,
    // Bytes 56..58: e_phnum = 2.
    0x02, 0x00,
    // Bytes 58..60: e_shentsize = 0.
    0x00, 0x00,
    // Bytes 60..62: e_shnum = 0.
    0x00, 0x00,
    // Bytes 62..64: e_shstrndx = 0.
    0x00, 0x00,

    // PH 0: PT_LOAD .text — vaddr 0x0040_1000, memsz 0x10.
    0x01, 0x00, 0x00, 0x00,
    0x05, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // PH 1: PT_LOAD .data — vaddr 0x0040_1008 (OVERLAPS .text!),
    // memsz 0x20.
    0x01, 0x00, 0x00, 0x00,
    0x06, 0x00, 0x00, 0x00,
    0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr = 0x0040_1008  (the overlap)
    0x08, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x08, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // Pad 0xB0..0x100 (80 bytes).
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // PT_LOAD #0 payload (16 bytes 0xAA) — 0x100..0x110.
    0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
    0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,

    // PT_LOAD #1 payload (8 bytes 0xBB) — 0x110..0x118.
    0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb, 0xbb,
];

// ---------------------------------------------------------------------------
// SPAWN fixture (#521)
// ---------------------------------------------------------------------------
//
// `SPAWN_ELF` is the smallest static binary the spawn pipeline can
// drive end-to-end: one PT_LOAD segment carrying the x86_64
// instruction bytes for `write(1, "hi", 2)` followed by
// `exit_group(0)` plus the two-byte string "hi" packed at the tail
// of the .text segment. Tier-1 of #521 stops at the entry-point jump
// — the syscall layer (#473) hasn't landed yet, so the first
// `syscall` instruction will trap. What this fixture proves at
// load time is:
//
//   * A single-PT_LOAD static binary parses + loads cleanly into the
//     `AddressSpace`.
//   * The `Process::spawn` call sets up the initial stack with argc=1
//     (argv[0] = `/bin/spawn`), argv terminator, envp terminator,
//     and the seven-entry auxv (AT_PHDR / AT_PHENT / AT_PHNUM /
//     AT_PAGESZ / AT_ENTRY / AT_RANDOM / AT_NULL).
//   * The trampoline's `setup_x86_64` produces a populated
//     `IretqFrame` with `rip = 0x40_1000` (= the entry point).
//
// The actual ring-3 jump is gated behind the
// `TrampolineError::NotYetImplemented` return until #526 ships the
// GDT/TSS scaffolding — the assertion is structural (the spawn
// pipeline reaches the trampoline doorstep without panicking).
//
// Instruction bytes (little-endian x86_64):
//
//   0x40_1000:  b8 01 00 00 00       mov  eax, 1            ; sys_write
//   0x40_1005:  bf 01 00 00 00       mov  edi, 1            ; fd = stdout
//   0x40_100a:  48 8d 35 0d 00 00 00 lea  rsi, [rip+0xd]   ; -> "hi" at 0x40_101e
//                                                          ;    rip after lea = 0x40_1011
//                                                          ;    + 0xd = 0x40_101e
//   0x40_1011:  ba 02 00 00 00       mov  edx, 2            ; len = 2
//   0x40_1016:  0f 05                syscall                ; trap (no syscall table)
//   0x40_1018:  b8 e7 00 00 00       mov  eax, 231          ; sys_exit_group
//   0x40_101d:  31 ff                xor  edi, edi          ; status = 0
//   ; (would be 0x40_101f: 0f 05 syscall, but we stop emitting; the
//   ;  above is enough to exercise the entry-point invoke)
//   0x40_101e:  68 69                ascii "hi"             ; write(2) buffer
//
// Total .text payload: 32 bytes (0x20). One PT_LOAD entry, no
// PT_INTERP (static binary), one PT_GNU_STACK (NX hint).
//
// Layout (256 bytes total):
//   File header           : 0x000..0x040  (64 bytes)
//   Program header 0      : 0x040..0x078  (PT_LOAD .text — 0x20 file/mem)
//   Program header 1      : 0x078..0x0B0  (PT_GNU_STACK)
//   Pad to 0x100          : zero-fill
//   PT_LOAD #0 payload    : 0x100..0x120  (32 bytes — instructions + "hi")
//   Pad to 0x140          : zero-fill
//
// Why ProgramHeader 0's `p_flags = PF_R | PF_W | PF_X` would be a W^X
// violation? It would. We use `PF_R | PF_X` (5) — tier-1 binaries
// store the "hi" string inside .text (rodata co-located with code) so
// it's still readable + executable but never written. Real toolchains
// split rodata into its own segment; the fixture cuts the corner so
// we keep one PT_LOAD.
pub const SPAWN_ELF: &[u8] = &[
    // -------- ELF64 file header (offsets 0x00..0x40) --------
    // e_ident: magic + class=64 + data=LE + version=1 + ABI=SYSV.
    0x7f, 0x45, 0x4c, 0x46,
    0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_type = ET_EXEC, e_machine = EM_X86_64, e_version = 1.
    0x02, 0x00, 0x3e, 0x00, 0x01, 0x00, 0x00, 0x00,
    // e_entry = 0x0040_1000.
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // e_phoff = 0x40, e_shoff = 0, e_flags = 0.
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
    // e_ehsize = 64, e_phentsize = 56, e_phnum = 2, rest = 0.
    0x40, 0x00,
    0x38, 0x00,
    0x02, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Program header 0: PT_LOAD .text (0x40..0x78) --------
    // p_type = PT_LOAD.
    0x01, 0x00, 0x00, 0x00,
    // p_flags = PF_R | PF_X.
    0x05, 0x00, 0x00, 0x00,
    // p_offset = 0x100.
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_vaddr = 0x0040_1000.
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_paddr = 0x0040_1000.
    0x00, 0x10, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_filesz = 0x20 (32 bytes — instructions + "hi" string).
    0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_memsz = 0x20.
    0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 0x1000.
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Program header 1: PT_GNU_STACK (0x78..0xB0) --------
    // p_type = PT_GNU_STACK.
    0x51, 0xe5, 0x74, 0x64,
    // p_flags = PF_R | PF_W (NX stack).
    0x06, 0x00, 0x00, 0x00,
    // p_offset / p_vaddr / p_paddr / p_filesz / p_memsz = 0.
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // p_align = 0x10.
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,

    // -------- Pad 0xB0..0x100 (80 bytes) --------
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,

    // -------- PT_LOAD #0 payload (32 bytes) — 0x100..0x120 --------
    // mov eax, 1                — b8 01 00 00 00
    0xb8, 0x01, 0x00, 0x00, 0x00,
    // mov edi, 1                — bf 01 00 00 00
    0xbf, 0x01, 0x00, 0x00, 0x00,
    // lea rsi, [rip+0xd]        — 48 8d 35 0d 00 00 00
    0x48, 0x8d, 0x35, 0x0d, 0x00, 0x00, 0x00,
    // mov edx, 2                — ba 02 00 00 00
    0xba, 0x02, 0x00, 0x00, 0x00,
    // syscall                   — 0f 05
    0x0f, 0x05,
    // mov eax, 231 (sys_exit_group) — b8 e7 00 00 00
    0xb8, 0xe7, 0x00, 0x00, 0x00,
    // xor edi, edi              — 31 ff
    0x31, 0xff,
    // ascii "hi"                — 68 69
    0x68, 0x69,

    // -------- Pad 0x120..0x140 (32 bytes) — slice tail --------
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
];
