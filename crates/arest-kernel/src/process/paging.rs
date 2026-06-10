// crates/arest-kernel/src/process/paging.rs
//
// The #527 page-table install: build a per-process 4-level page-table
// tree that keeps the kernel's boot-time identity view (supervisor)
// and adds USER-accessible 4 KiB mappings for the process image —
// each ELF segment's [vaddr, vaddr+mem_size) mapped to its heap
// backing, plus the initial stack and the AT_RANDOM page at their
// identity addresses.
//
// Why this exists: tier-1 ran ring 3 directly on the firmware's
// identity tables. Two fatal consequences, both observed in the #527
// QEMU smoke: (1) every page is supervisor-only, so the FIRST user
// instruction fetch faults (#PF → unwired vector → DOUBLE FAULT);
// (2) the ELF loader places segment bytes in HEAP allocations while
// the iretq jumps to the segment's literal ELF vaddr — under pure
// identity mapping those are different physical locations, so even a
// USER-flipped identity page would execute whatever garbage RAM sits
// at the vaddr. The fix is the classical one: a process CR3 whose
// user ranges TRANSLATE vaddr → heap-backing phys.
//
// Sharing/cloning policy
// ----------------------
// The builder starts from a verbatim copy of the boot PML4 (so the
// kernel keeps its whole identity view under the process CR3 — ISRs,
// the syscall entry, kernel heap, MMIO all resolve unchanged), then
// performs copy-on-descend: any boot-owned interior table is CLONED
// into a process-owned `Box<PageTable>` before being modified, and
// any huge leaf (1 GiB PDPTE / 2 MiB PDE) covering a user range is
// SPLIT into the next-smaller unit, preserving the original flags
// for the non-user remainder. Untouched subtrees stay physically
// shared with the boot tables — read-only sharing of 'static
// firmware tables, never mutated.
//
// USER_ACCESSIBLE must be set at EVERY level along a user path
// (access is the AND of U bits down the walk); interior entries on
// user paths are made permissive (P|W|U) and the LEAF carries the
// real permission (writable per SegmentPerm; NX deliberately not set
// — tier-1 doesn't manage EFER.NXE, and setting NX with NXE=0 is a
// reserved-bit #PF).
//
// Identity assumptions (UEFI tier-1): virt == phys for kernel heap,
// so a `Box<PageTable>`'s address IS the physical address the parent
// entry needs, and a heap backing pointer IS the physical frame of
// the user page. The host tests construct synthetic boot trees and
// verify structure purely in memory — only `activate()` touches CR3
// and is UEFI-gated.

use alloc::boxed::Box;
use alloc::vec::Vec;
use x86_64::structures::paging::page_table::PageTableFlags as F;
use x86_64::structures::paging::PageTable;
use x86_64::PhysAddr;

/// 4 KiB — the only mapping granularity user ranges use.
const PAGE_SIZE: u64 = 4096;

/// One user-visible mapping request: `len` bytes at `vaddr` backed by
/// the physically-contiguous range starting at `phys` (under tier-1's
/// identity heap, the backing allocation's address). All three must
/// be 4 KiB-aligned (`len` rounded up by the caller — segment sizes
/// already are).
#[derive(Debug, Clone, Copy)]
pub struct UserMapping {
    pub vaddr: u64,
    pub phys: u64,
    pub len: u64,
    pub writable: bool,
}

/// Why a build failed. Alignment is the caller's contract; the
/// others are structural impossibilities for well-formed inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingError {
    /// vaddr / phys / len not 4 KiB-aligned (or len == 0).
    Unaligned,
    /// The 48-bit canonical range was exceeded.
    VaddrOutOfRange,
    /// A walk met an entry shape it can't transform. Currently
    /// unreachable — non-present interiors get fresh tables (the
    /// mmap territory at 0x7000_0000_0000 is absent from the boot
    /// view by construction) and huge leaves split. Kept for future
    /// walk shapes that CAN refuse.
    UnexpectedEntry,
}

/// A built process page-table tree. `root` is the PML4; `subtables`
/// own every interior table this process allocated (clones + splits
/// + fresh). Dropping this frees them — callers that `activate()` it
/// must keep the value alive for as long as the CR3 points at it
/// (the spawn path's iretq diverges, which keeps the owning Process
/// frame alive forever — see process.rs).
pub struct ProcessPageTables {
    root: Box<PageTable>,
    subtables: Vec<Box<PageTable>>,
}

impl core::fmt::Debug for ProcessPageTables {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProcessPageTables")
            .field("root_phys", &self.root_phys())
            .field("owned_tables", &self.subtables.len())
            .finish()
    }
}

impl ProcessPageTables {
    /// Physical address of the PML4 (identity: the Box's address).
    pub fn root_phys(&self) -> u64 {
        &*self.root as *const PageTable as u64
    }

    /// Borrow the root for inspection (tests, diagnostics).
    pub fn root(&self) -> &PageTable {
        &self.root
    }

    /// Number of process-owned interior tables (tests/diagnostics).
    pub fn owned_table_count(&self) -> usize {
        self.subtables.len()
    }

    /// Resolve a process-owned table by its address, if this tree
    /// allocated it. Tests use this to follow entries structurally
    /// without dereferencing raw physical addresses.
    pub fn owned_table(&self, addr: u64) -> Option<&PageTable> {
        if addr == self.root_phys() {
            return Some(&self.root);
        }
        self.subtables
            .iter()
            .map(|b| &**b)
            .find(|t| (*t as *const PageTable as u64) == addr)
    }

    /// Load this tree's root into CR3. UEFI x86_64 only — the host
    /// has no guest CR3 to write.
    ///
    /// # Safety
    /// The tree must keep the kernel's live view intact (guaranteed
    /// by `build`'s verbatim-copy + copy-on-descend policy) and must
    /// outlive every instruction executed under it.
    #[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
    pub unsafe fn activate(&self) {
        use x86_64::registers::control::{Cr3, Cr3Flags};
        use x86_64::structures::paging::PhysFrame;
        let frame = PhysFrame::containing_address(PhysAddr::new(self.root_phys()));
        unsafe { Cr3::write(frame, Cr3Flags::empty()) };
    }

    /// Extend an already-built (possibly CR3-ACTIVE) tree with one
    /// more user mapping — the post-spawn anonymous-mmap path
    /// (#497-c). Same validation contract as `build`.
    ///
    /// Live-tree safety: a new translation flips entries from
    /// non-present to present, which the TLB never caches (SDM
    /// 4.10.2.3), so no flush is needed for the fresh-table case.
    /// Splits / permission bumps of PRESENT entries DO leave stale
    /// TLB + paging-structure-cache state, so the UEFI arm flushes
    /// the mapped range per page regardless — cheap at mmap rates
    /// and unconditionally correct.
    pub fn map_additional(&mut self, m: &UserMapping) -> Result<(), PagingError> {
        let end = check_mapping(m)?;
        let mut page = m.vaddr;
        let mut phys = m.phys;
        while page < end {
            map_user_page(self, page, phys, m.writable)?;
            page += PAGE_SIZE;
            phys += PAGE_SIZE;
        }
        #[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
        {
            let mut page = m.vaddr;
            while page < end {
                x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(page));
                page += PAGE_SIZE;
            }
        }
        Ok(())
    }
}

/// Shared per-mapping validation: alignment + 48-bit canonical range.
/// Returns the exclusive end address on success.
fn check_mapping(m: &UserMapping) -> Result<u64, PagingError> {
    if m.len == 0 || m.vaddr % PAGE_SIZE != 0 || m.phys % PAGE_SIZE != 0 || m.len % PAGE_SIZE != 0
    {
        return Err(PagingError::Unaligned);
    }
    let end = m
        .vaddr
        .checked_add(m.len)
        .ok_or(PagingError::VaddrOutOfRange)?;
    if end > 1u64 << 48 {
        return Err(PagingError::VaddrOutOfRange);
    }
    Ok(end)
}

/// Flags every interior entry on a user path gets: permissive parent,
/// restrictive leaf. WRITABLE at interior level is required for ANY
/// user write to leaves below it.
fn interior_user_flags() -> F {
    F::PRESENT | F::WRITABLE | F::USER_ACCESSIBLE
}

fn leaf_user_flags(writable: bool) -> F {
    let mut f = F::PRESENT | F::USER_ACCESSIBLE;
    if writable {
        f |= F::WRITABLE;
    }
    f
}

/// Build a process tree from `boot_root` (the live CR3's PML4 under
/// identity assumptions — or a synthetic fixture in tests) plus the
/// user mappings.
pub fn build(
    boot_root: &PageTable,
    mappings: &[UserMapping],
) -> Result<ProcessPageTables, PagingError> {
    // Verbatim PML4 copy: the kernel view, shared at PDPT depth.
    let mut root: Box<PageTable> = Box::new(PageTable::new());
    for (i, entry) in boot_root.iter().enumerate() {
        root[i] = entry.clone();
    }
    let mut tree = ProcessPageTables {
        root,
        subtables: Vec::new(),
    };

    for m in mappings {
        let end = check_mapping(m)?;
        let mut page = m.vaddr;
        let mut phys = m.phys;
        while page < end {
            map_user_page(&mut tree, page, phys, m.writable)?;
            page += PAGE_SIZE;
            phys += PAGE_SIZE;
        }
    }
    Ok(tree)
}

/// Read an entry's raw target address + flags. (PageTableEntry's
/// `addr()` masks the flag bits for us.)
fn entry_parts(table: &PageTable, idx: usize) -> (u64, F) {
    let e = &table[idx];
    (e.addr().as_u64(), e.flags())
}

/// Ensure `tree.root[..]`'s path for `vaddr` descends through
/// process-owned tables down to the PT, splitting huge leaves and
/// cloning boot-shared interiors as it goes, then write the 4 KiB
/// user PTE.
fn map_user_page(
    tree: &mut ProcessPageTables,
    vaddr: u64,
    phys: u64,
    writable: bool,
) -> Result<(), PagingError> {
    let pml4_i = ((vaddr >> 39) & 0x1ff) as usize;
    let pdpt_i = ((vaddr >> 30) & 0x1ff) as usize;
    let pd_i = ((vaddr >> 21) & 0x1ff) as usize;
    let pt_i = ((vaddr >> 12) & 0x1ff) as usize;

    // ── PML4 → PDPT ────────────────────────────────────────────────
    let (pml4e_addr, pml4e_flags) = entry_parts(&tree.root, pml4_i);
    let pdpt_addr = if !pml4e_flags.contains(F::PRESENT) {
        // Absent from the boot view (the mmap territory at
        // 0x7000_0000_0000 by construction): fresh process-owned
        // PDPT, private from birth — nothing to clone or share.
        let addr = push_owned(tree, Box::new(PageTable::new()));
        tree.root[pml4_i].set_addr(PhysAddr::new(addr), interior_user_flags());
        addr
    } else if tree.owned_table(pml4e_addr).is_some() {
        pml4e_addr
    } else {
        // Boot-shared PDPT: clone before touching.
        // SAFETY (UEFI): identity mapping makes the entry's addr a
        // dereferencable kernel VA for a live 'static firmware table.
        // On the host, tests only hand in synthetic trees whose
        // entry addrs point at test-owned boxes, same contract.
        let src: &PageTable = unsafe { &*(pml4e_addr as *const PageTable) };
        let clone = clone_table(src);
        let addr = push_owned(tree, clone);
        tree.root[pml4_i].set_addr(
            PhysAddr::new(addr),
            pml4e_flags | interior_user_flags(),
        );
        addr
    };
    // U must be on the path even when the table was already owned.
    let (a, f) = entry_parts(&tree.root, pml4_i);
    tree.root[pml4_i].set_addr(PhysAddr::new(a), f | interior_user_flags());

    // ── PDPT → PD (split 1 GiB leaves) ─────────────────────────────
    let (pdpte_addr, pdpte_flags) = read_owned(tree, pdpt_addr, pdpt_i);
    let pd_addr = if !pdpte_flags.contains(F::PRESENT) {
        // Absent: fresh process-owned PD (same rationale as PML4).
        let addr = push_owned(tree, Box::new(PageTable::new()));
        write_owned(tree, pdpt_addr, pdpt_i, addr, interior_user_flags());
        addr
    } else if pdpte_flags.contains(F::HUGE_PAGE) {
        // 1 GiB leaf → 512 × 2 MiB leaves preserving flags.
        let mut pd = Box::new(PageTable::new());
        let base = pdpte_addr;
        let leaf_flags = pdpte_flags; // keep HUGE_PAGE: 2 MiB leaves
        for (i, e) in pd.iter_mut().enumerate() {
            e.set_addr(
                PhysAddr::new(base + (i as u64) * (2 * 1024 * 1024)),
                leaf_flags,
            );
        }
        let addr = push_owned(tree, pd);
        write_owned(
            tree,
            pdpt_addr,
            pdpt_i,
            addr,
            (pdpte_flags - F::HUGE_PAGE) | interior_user_flags(),
        );
        addr
    } else if tree.owned_table(pdpte_addr).is_some() {
        bump_user(tree, pdpt_addr, pdpt_i);
        pdpte_addr
    } else {
        let src: &PageTable = unsafe { &*(pdpte_addr as *const PageTable) };
        let clone = clone_table(src);
        let addr = push_owned(tree, clone);
        write_owned(
            tree,
            pdpt_addr,
            pdpt_i,
            addr,
            pdpte_flags | interior_user_flags(),
        );
        addr
    };

    // ── PD → PT (split 2 MiB leaves) ───────────────────────────────
    let (pde_addr, pde_flags) = read_owned(tree, pd_addr, pd_i);
    let pt_addr = if !pde_flags.contains(F::PRESENT) {
        // Absent: fresh process-owned PT (same rationale as PML4).
        let addr = push_owned(tree, Box::new(PageTable::new()));
        write_owned(tree, pd_addr, pd_i, addr, interior_user_flags());
        addr
    } else if pde_flags.contains(F::HUGE_PAGE) {
        // 2 MiB leaf → 512 × 4 KiB PTEs. HUGE_PAGE must be DROPPED in
        // PTEs (bit 7 is PAT at PT level).
        let mut pt = Box::new(PageTable::new());
        let base = pde_addr;
        let leaf_flags = pde_flags - F::HUGE_PAGE;
        for (i, e) in pt.iter_mut().enumerate() {
            e.set_addr(PhysAddr::new(base + (i as u64) * PAGE_SIZE), leaf_flags);
        }
        let addr = push_owned(tree, pt);
        write_owned(
            tree,
            pd_addr,
            pd_i,
            addr,
            (pde_flags - F::HUGE_PAGE) | interior_user_flags(),
        );
        addr
    } else if tree.owned_table(pde_addr).is_some() {
        bump_user(tree, pd_addr, pd_i);
        pde_addr
    } else {
        let src: &PageTable = unsafe { &*(pde_addr as *const PageTable) };
        let clone = clone_table(src);
        let addr = push_owned(tree, clone);
        write_owned(tree, pd_addr, pd_i, addr, pde_flags | interior_user_flags());
        addr
    };

    // ── PT: the user leaf ──────────────────────────────────────────
    write_owned(tree, pt_addr, pt_i, phys, leaf_user_flags(writable));
    Ok(())
}

fn clone_table(src: &PageTable) -> Box<PageTable> {
    let mut t = Box::new(PageTable::new());
    for (i, e) in src.iter().enumerate() {
        t[i] = e.clone();
    }
    t
}

fn push_owned(tree: &mut ProcessPageTables, t: Box<PageTable>) -> u64 {
    let addr = &*t as *const PageTable as u64;
    tree.subtables.push(t);
    addr
}

/// Read entry `idx` of the process-owned table at `addr`.
fn read_owned(tree: &ProcessPageTables, addr: u64, idx: usize) -> (u64, F) {
    let t = tree
        .owned_table(addr)
        .expect("read_owned: caller guarantees process ownership");
    entry_parts(t, idx)
}

/// Write entry `idx` of the process-owned table at `addr`.
fn write_owned(tree: &mut ProcessPageTables, addr: u64, idx: usize, target: u64, flags: F) {
    let root_addr = tree.root_phys();
    let t: &mut PageTable = if addr == root_addr {
        &mut tree.root
    } else {
        tree.subtables
            .iter_mut()
            .map(|b| &mut **b)
            .find(|t| (*t as *const PageTable as u64) == addr)
            .expect("write_owned: caller guarantees process ownership")
    };
    t[idx].set_addr(PhysAddr::new(target), flags);
}

/// OR `interior_user_flags` into an existing owned entry.
fn bump_user(tree: &mut ProcessPageTables, table_addr: u64, idx: usize) {
    let (a, f) = read_owned(tree, table_addr, idx);
    write_owned(tree, table_addr, idx, a, f | interior_user_flags());
}

/// Build the tree for the LIVE boot CR3 (identity view) — UEFI only.
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
pub fn build_for_current(
    mappings: &[UserMapping],
) -> Result<ProcessPageTables, PagingError> {
    use x86_64::registers::control::Cr3;
    let (frame, _) = Cr3::read();
    // SAFETY: identity mapping — the live PML4's physical frame is
    // dereferencable at the same address; firmware tables are 'static.
    let boot_root: &PageTable =
        unsafe { &*(frame.start_address().as_u64() as *const PageTable) };
    build(boot_root, mappings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic boot tree: PML4[0] → PDPT whose entry 0 is a 1 GiB
    /// identity huge leaf at phys 0 (supervisor, global). Everything
    /// else non-present. Returned boxes must outlive the built tree
    /// (the builder dereferences their addresses on clone).
    fn synthetic_boot() -> (Box<PageTable>, Box<PageTable>) {
        let mut pdpt = Box::new(PageTable::new());
        pdpt[0].set_addr(
            PhysAddr::new(0),
            F::PRESENT | F::WRITABLE | F::HUGE_PAGE | F::GLOBAL,
        );
        let mut pml4 = Box::new(PageTable::new());
        let pdpt_addr = &*pdpt as *const PageTable as u64;
        pml4[0].set_addr(PhysAddr::new(pdpt_addr), F::PRESENT | F::WRITABLE);
        (pml4, pdpt)
    }

    /// Follow a vaddr down the built tree, returning the four
    /// (addr, flags) tuples for PML4E/PDPTE/PDE/PTE.
    fn walk(tree: &ProcessPageTables, vaddr: u64) -> [(u64, F); 4] {
        let idx = [
            ((vaddr >> 39) & 0x1ff) as usize,
            ((vaddr >> 30) & 0x1ff) as usize,
            ((vaddr >> 21) & 0x1ff) as usize,
            ((vaddr >> 12) & 0x1ff) as usize,
        ];
        let mut out = [(0u64, F::empty()); 4];
        let mut table = tree.root();
        for level in 0..4 {
            let e = &table[idx[level]];
            out[level] = (e.addr().as_u64(), e.flags());
            if level < 3 {
                table = tree
                    .owned_table(e.addr().as_u64())
                    .expect("user path must descend through owned tables");
            }
        }
        out
    }

    /// The canonical case: one 3-page user mapping at an ELF-style
    /// vaddr inside the identity GiB. The walk must end at user PTEs
    /// pointing at the requested phys, with USER set at every level,
    /// and the huge leaves split around it must preserve identity.
    #[test]
    fn maps_user_range_with_split_and_user_path() {
        let (pml4, _pdpt_keepalive) = synthetic_boot();
        let m = UserMapping {
            vaddr: 0x20_0000,
            phys: 0x5555_5000,
            len: 0x3000,
            writable: true,
        };
        let tree = build(&pml4, &[m]).expect("build");

        let path = walk(&tree, 0x20_0000);
        for (lvl, (_, f)) in path.iter().enumerate() {
            assert!(
                f.contains(F::USER_ACCESSIBLE),
                "level {lvl} missing USER: {f:?}"
            );
            assert!(f.contains(F::PRESENT), "level {lvl} missing PRESENT");
        }
        // Leaf: requested phys, writable, NOT huge.
        let (pte_addr, pte_flags) = path[3];
        assert_eq!(pte_addr, 0x5555_5000);
        assert!(pte_flags.contains(F::WRITABLE));
        assert!(!pte_flags.contains(F::HUGE_PAGE));
        // Page 2 of the mapping advances phys by 2 pages.
        let p2 = walk(&tree, 0x20_2000);
        assert_eq!(p2[3].0, 0x5555_5000 + 0x2000);

        // The split preserved identity around the user range: the
        // page AFTER the mapping (0x203000) must still point at its
        // identity phys, supervisor-only.
        let after = walk_pt_only(&tree, 0x20_3000);
        assert_eq!(after.0, 0x20_3000, "identity preserved after range");
        assert!(
            !after.1.contains(F::USER_ACCESSIBLE),
            "non-user page must stay supervisor"
        );
        // And the 2 MiB chunk BEFORE ours is still a huge identity
        // leaf (split touched only chunk #1).
        let pd_addr = walk(&tree, 0x20_0000)[1].0;
        let pd = tree.owned_table(pd_addr).expect("owned PD");
        let (chunk0_addr, chunk0_flags) = (pd[0].addr().as_u64(), pd[0].flags());
        assert_eq!(chunk0_addr, 0);
        assert!(chunk0_flags.contains(F::HUGE_PAGE), "chunk 0 still huge");
        assert!(!chunk0_flags.contains(F::USER_ACCESSIBLE));
    }

    /// Walk only to the PTE, without asserting user-ownership of the
    /// path (used for neighbours that share split tables).
    fn walk_pt_only(tree: &ProcessPageTables, vaddr: u64) -> (u64, F) {
        let path = walk(tree, vaddr);
        path[3]
    }

    /// PML4 entries outside the user path must be byte-identical to
    /// the boot root (the kernel view is shared, not rebuilt).
    #[test]
    fn untouched_pml4_entries_are_verbatim_copies() {
        let (pml4, _keep) = synthetic_boot();
        let m = UserMapping {
            vaddr: 0x20_0000,
            phys: 0x9000,
            len: 0x1000,
            writable: false,
        };
        let tree = build(&pml4, &[m]).expect("build");
        for i in 1..512 {
            assert_eq!(
                tree.root()[i].addr().as_u64(),
                pml4[i].addr().as_u64(),
                "entry {i} addr drifted"
            );
            assert_eq!(tree.root()[i].flags(), pml4[i].flags(), "entry {i} flags");
        }
    }

    /// A read-only mapping must yield a non-writable PTE while the
    /// interior path stays writable (parent W gates child W).
    #[test]
    fn read_only_mapping_clears_leaf_writable() {
        let (pml4, _keep) = synthetic_boot();
        let m = UserMapping {
            vaddr: 0x40_0000,
            phys: 0x7000,
            len: 0x1000,
            writable: false,
        };
        let tree = build(&pml4, &[m]).expect("build");
        let path = walk(&tree, 0x40_0000);
        assert!(!path[3].1.contains(F::WRITABLE), "leaf must be RO");
        assert!(path[2].1.contains(F::WRITABLE), "interior stays writable");
    }

    /// Alignment contract: unaligned vaddr / phys / len and zero len
    /// are rejected.
    #[test]
    fn unaligned_inputs_are_rejected() {
        let (pml4, _keep) = synthetic_boot();
        for m in [
            UserMapping { vaddr: 0x100, phys: 0, len: 0x1000, writable: true },
            UserMapping { vaddr: 0x1000, phys: 0x10, len: 0x1000, writable: true },
            UserMapping { vaddr: 0x1000, phys: 0, len: 0x800, writable: true },
            UserMapping { vaddr: 0x1000, phys: 0, len: 0, writable: true },
        ] {
            assert_eq!(build(&pml4, &[m]).unwrap_err(), PagingError::Unaligned);
        }
    }

    /// Two mappings in the SAME 2 MiB chunk share one split PT — the
    /// second mapping must not re-split or clobber the first.
    #[test]
    fn two_mappings_share_one_split_pt() {
        let (pml4, _keep) = synthetic_boot();
        let a = UserMapping { vaddr: 0x20_0000, phys: 0xa000, len: 0x1000, writable: true };
        let b = UserMapping { vaddr: 0x20_5000, phys: 0xb000, len: 0x1000, writable: true };
        let tree = build(&pml4, &[a, b]).expect("build");
        assert_eq!(walk(&tree, 0x20_0000)[3].0, 0xa000);
        assert_eq!(walk(&tree, 0x20_5000)[3].0, 0xb000);
        // Same PT table on both paths.
        assert_eq!(walk(&tree, 0x20_0000)[2].0, walk(&tree, 0x20_5000)[2].0);
    }

    /// The mmap territory (0x7000_0000_0000, PML4 slot 224) is ABSENT
    /// from the boot identity view — the builder must allocate fresh
    /// process-owned interior tables down the whole path rather than
    /// erroring (#497-c: anonymous mmap backing lives here).
    #[test]
    fn maps_into_absent_pml4_slot_with_fresh_tables() {
        let (pml4, _keep) = synthetic_boot();
        let m = UserMapping {
            vaddr: 0x7000_0000_0000,
            phys: 0xc000,
            len: 0x2000,
            writable: true,
        };
        let tree = build(&pml4, &[m]).expect("absent territory must build");
        let path = walk(&tree, 0x7000_0000_0000);
        for (lvl, (_, f)) in path.iter().enumerate() {
            assert!(f.contains(F::PRESENT), "level {lvl} missing PRESENT");
            assert!(
                f.contains(F::USER_ACCESSIBLE),
                "level {lvl} missing USER: {f:?}"
            );
        }
        assert_eq!(path[3].0, 0xc000);
        assert_eq!(walk(&tree, 0x7000_0000_1000)[3].0, 0xd000);
        // The boot view is untouched: PML4 slot 224 was and stays
        // empty in the SOURCE tree (the process tree owns the new
        // subtree privately).
        assert!(!pml4[224].flags().contains(F::PRESENT));
    }

    /// `map_additional` extends an already-built tree — the post-spawn
    /// mmap path. The original mappings stay intact and the new range
    /// resolves with a full USER path.
    #[test]
    fn map_additional_extends_built_tree() {
        let (pml4, _keep) = synthetic_boot();
        let seg = UserMapping { vaddr: 0x20_0000, phys: 0xa000, len: 0x1000, writable: true };
        let mut tree = build(&pml4, &[seg]).expect("build");
        let mm = UserMapping {
            vaddr: 0x7000_0000_0000,
            phys: 0xe000,
            len: 0x1000,
            writable: true,
        };
        tree.map_additional(&mm).expect("extension must map");
        // Old mapping intact.
        assert_eq!(walk(&tree, 0x20_0000)[3].0, 0xa000);
        // New mapping live, full USER path.
        let path = walk(&tree, 0x7000_0000_0000);
        assert_eq!(path[3].0, 0xe000);
        for (lvl, (_, f)) in path.iter().enumerate() {
            assert!(
                f.contains(F::USER_ACCESSIBLE) && f.contains(F::PRESENT),
                "level {lvl} not a live user path: {f:?}"
            );
        }
        // Alignment contract holds on the extension path too.
        let bad = UserMapping { vaddr: 0x123, phys: 0, len: 0x1000, writable: true };
        assert_eq!(tree.map_additional(&bad).unwrap_err(), PagingError::Unaligned);
    }
}
