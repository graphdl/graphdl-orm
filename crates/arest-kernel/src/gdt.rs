// crates/arest-kernel/src/gdt.rs
//
// Global Descriptor Table + Task State Segment for x86_64 long mode.
//
// Long-mode protection checks still consult the GDT for the code and
// data segment descriptors every time the CPU changes privilege
// level (int, iret, syscall, sysret). The bootloader gave us a
// scratch GDT that works well enough to run ring-0 code, but it does
// NOT include a TSS, which means double-faults have nowhere to land
// and end up triple-faulting the CPU.
//
// This module builds our own GDT with:
//   - one 64-bit code segment (ring 0),
//   - one TSS containing a dedicated Interrupt Stack Table (IST)
//     entry for the double-fault handler.
//
// Loading the TSS gives `interrupts::init_idt` somewhere to point
// the double-fault vector's IST index, so a stack overflow in a
// nested interrupt doesn't crash the machine.

use core::mem::MaybeUninit;
use spin::Once;
use x86_64::VirtAddr;
use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::{CS, Segment};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

/// IST entry index reserved for the double-fault handler. Chosen
/// arbitrarily — any unused IST slot [0..7] works; we pick 0 to
/// keep later handlers free to claim 1+.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

/// Size of the double-fault stack. 4 KiB is the minimum a sane
/// handler needs; stack overflows are the textbook cause of
/// double-faults so the handler itself must not require much.
const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 5;

/// Dedicated stack for the double-fault handler. Stored as
/// `MaybeUninit` to avoid cost of zeroing a 20 KiB buffer at load
/// time — the IST just needs a valid range of memory; content is
/// irrelevant until something faults.
#[used]
static mut DOUBLE_FAULT_STACK: MaybeUninit<[u8; DOUBLE_FAULT_STACK_SIZE]> =
    MaybeUninit::uninit();

/// TSS instance populated once at boot and left alive for the rest
/// of the kernel's lifetime. Stored behind `Once` so the GDT entry
/// can point at a long-lived `&'static TaskStateSegment`.
static TSS: Once<TaskStateSegment> = Once::new();

/// GDT + the code-segment / TSS selectors picked after load. Built
/// lazily so both the TSS and the GDT that references it live long
/// enough — the GDT descriptor contains a pointer to the TSS and
/// both must outlive every segment switch.
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

struct Selectors {
    code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

/// Initialise the GDT + TSS. Call once, early in boot, before any
/// interrupt handler can fire. After this returns, the CPU is
/// running with our GDT + our code-segment descriptor, and the
/// double-fault IST entry points at DOUBLE_FAULT_STACK.
pub fn init() {
    let tss = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            let stack_start = VirtAddr::from_ptr(core::ptr::addr_of!(DOUBLE_FAULT_STACK));
            stack_start + DOUBLE_FAULT_STACK_SIZE as u64
        };
        tss
    });

    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.append(Descriptor::kernel_code_segment());
        let tss_selector = gdt.append(Descriptor::tss_segment(tss));
        (
            gdt,
            Selectors {
                code_selector,
                tss_selector,
            },
        )
    });

    gdt.load();
    unsafe {
        CS::set_reg(selectors.code_selector);
        load_tss(selectors.tss_selector);
    }
}
