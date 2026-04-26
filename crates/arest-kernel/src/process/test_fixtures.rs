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
