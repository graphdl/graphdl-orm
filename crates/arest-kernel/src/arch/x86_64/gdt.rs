// crates/arest-kernel/src/arch/x86_64/gdt.rs
//
// Global Descriptor Table + Task State Segment for x86_64 long mode
// with ring-3 support. Lives under `arch/x86_64/` (#344 step 2); the
// kernel body reaches it through `crate::arch::gdt`.
//
// SYSCALL/SYSRETQ require a specific descriptor ordering in the GDT
// because the CPU derives the user-mode selectors from STAR.SYSRET_CS
// by fixed offsets: +8 loads the user SS and +16 loads the 64-bit
// user CS. Any departure from this layout is a silent bug — syscall
// returns land on garbage segments.
//
// Layout (indices within the GDT):
//   idx 0  Null
//   idx 1  Kernel CS  (DPL=0, L=1)       <- STAR.SYSCALL_CS_SS
//   idx 2  Kernel SS  (DPL=0, W=1)       <-  +8
//   idx 3  User   CS32 (DPL=3, L=0)      <- STAR.SYSRET_CS_SS
//   idx 4  User   SS   (DPL=3, W=1)      <-  +8 (SYSRETQ loads SS from here)
//   idx 5  User   CS64 (DPL=3, L=1)      <- +16 (SYSRETQ loads CS from here)
//   idx 6  TSS  (16 bytes, spans two GDT slots)
//
// The TSS carries RSP0 = kernel stack top so CPU exceptions taken
// while running in CPL=3 automatically switch to a safe kernel stack
// before vectoring to the handler. Without RSP0 set, the first ring-3
// page fault triple-faults the box.

use core::mem::MaybeUninit;
use spin::Once;
use x86_64::VirtAddr;
use x86_64::instructions::segmentation::{DS, ES, FS, GS, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::{CS, Segment};
use x86_64::structures::gdt::{
    Descriptor, DescriptorFlags, GlobalDescriptorTable, SegmentSelector,
};
use x86_64::structures::tss::TaskStateSegment;

/// IST entry index reserved for the double-fault handler.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

/// Size of the double-fault stack. 20 KiB — enough for a sane panic
/// handler that needs to format and print a stack frame.
const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 5;

/// Size of the kernel ring-0 stack (TSS.RSP0). 16 KiB is plenty —
/// we park it in a dedicated static so the address is well-known.
const KERNEL_STACK_SIZE: usize = 4096 * 4;

/// Dedicated stack for the double-fault IST entry.
#[used]
static mut DOUBLE_FAULT_STACK: MaybeUninit<[u8; DOUBLE_FAULT_STACK_SIZE]> =
    MaybeUninit::uninit();

/// Dedicated stack for ring-0 exception delivery (TSS.RSP0). Used
/// whenever the CPU takes an exception from CPL=3 or via any IDT
/// gate that specifies an explicit stack switch.
#[used]
static mut KERNEL_STACK: MaybeUninit<[u8; KERNEL_STACK_SIZE]> = MaybeUninit::uninit();

/// TSS populated once at boot.
static TSS: Once<TaskStateSegment> = Once::new();

/// GDT + exported selectors. Populated by `init()`.
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

/// The set of segment selectors the rest of the kernel cares about
/// after `init()`. All are 16-bit selectors that can be loaded into
/// the segmentation registers directly.
#[derive(Clone, Copy, Debug)]
pub struct Selectors {
    pub kernel_cs: SegmentSelector,
    pub kernel_ss: SegmentSelector,
    pub user_cs32: SegmentSelector,
    pub user_ss:   SegmentSelector,
    pub user_cs64: SegmentSelector,
    pub tss:       SegmentSelector,
}

/// Kernel-stack top virtual address. Valid after `init()`. Used by
/// `syscall::init()` to seed the per-cpu KERNEL_RSP slot so the
/// SYSCALL trampoline has a safe stack to switch to.
pub fn kernel_stack_top() -> VirtAddr {
    let base = VirtAddr::from_ptr(core::ptr::addr_of!(KERNEL_STACK));
    base + KERNEL_STACK_SIZE as u64
}

/// Retrieve the selector set. Panics if called before `init()`.
pub fn selectors() -> Selectors {
    GDT.get().expect("gdt::init not called").1
}

/// Build the TSS + GDT and activate them. Must run once, early in
/// boot, before any handler can fire and before `syscall::init`.
pub fn init() {
    let tss = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();

        // Double-fault IST stack.
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            let base = VirtAddr::from_ptr(core::ptr::addr_of!(DOUBLE_FAULT_STACK));
            base + DOUBLE_FAULT_STACK_SIZE as u64
        };

        // RSP0 — kernel stack used for ring-3 -> ring-0 privilege
        // transitions through any IDT gate. Required before the
        // first iretq into ring 3.
        tss.privilege_stack_table[0] = kernel_stack_top();

        tss
    });

    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        // Order matters — STAR encodes selectors by position. See the
        // module-level comment for the fixed layout and offsets.
        let kernel_cs = gdt.append(Descriptor::kernel_code_segment());
        let kernel_ss = gdt.append(Descriptor::kernel_data_segment());
        // x86_64 0.15 does not expose a named constructor for the
        // 32-bit compat user code segment even though it has the
        // DescriptorFlags constant. Build it by hand — SYSCALL/SYSRETQ
        // require this entry at STAR.SYSRET_CS_SS so the +8 / +16
        // offsets land on the user SS and user 64-bit CS respectively.
        let user_cs32 = gdt.append(Descriptor::UserSegment(
            DescriptorFlags::USER_CODE32.bits(),
        ));
        let user_ss   = gdt.append(Descriptor::user_data_segment());
        let user_cs64 = gdt.append(Descriptor::user_code_segment());
        let tss_sel   = gdt.append(Descriptor::tss_segment(tss));
        (
            gdt,
            Selectors {
                kernel_cs,
                kernel_ss,
                user_cs32,
                user_ss,
                user_cs64,
                tss: tss_sel,
            },
        )
    });

    gdt.load();
    unsafe {
        // Reload every segmentation register so we drop any stale
        // selectors the bootloader left behind. SS / DS / ES / FS /
        // GS all get the kernel-data selector; only CS gets the
        // kernel-code selector via `set_reg`.
        CS::set_reg(selectors.kernel_cs);
        SS::set_reg(selectors.kernel_ss);
        DS::set_reg(selectors.kernel_ss);
        ES::set_reg(selectors.kernel_ss);
        FS::set_reg(SegmentSelector(0));
        GS::set_reg(SegmentSelector(0));
        load_tss(selectors.tss);
    }
}
