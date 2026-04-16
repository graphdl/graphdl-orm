// crates/arest-kernel/src/interrupts.rs
//
// Interrupt Descriptor Table setup. Starts minimal — just the
// breakpoint (#BP, int 3) and double-fault (#DF, int 8) vectors —
// and grows as we add timer (#180), keyboard (#181), and other
// device drivers.
//
// Breakpoint and double-fault are enough to prove the IDT plumbing
// works: `x86_64::instructions::interrupts::int3()` from the
// kernel will round-trip through our breakpoint handler and
// continue; any stack overflow or nested fault will route through
// the double-fault IST stack instead of triple-faulting.

use crate::gdt::DOUBLE_FAULT_IST_INDEX;
use crate::println;
use spin::Once;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

/// IDT instance populated once at boot and left alive for the rest
/// of the kernel's lifetime. `x86_64::structures::idt::InterruptDescriptorTable`
/// can be `load()`ed from a long-lived `&'static` — the `lidt`
/// instruction stores a pointer into the CPU's IDTR register, so
/// the table itself must not move after the load.
static IDT: Once<InterruptDescriptorTable> = Once::new();

/// Build the IDT and load it into the CPU. Call once, after
/// `gdt::init` — the double-fault entry references the IST index
/// registered in the GDT's TSS.
pub fn init_idt() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(DOUBLE_FAULT_IST_INDEX);
        }
        idt
    });
    idt.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{stack_frame:#?}");
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{stack_frame:#?}");
}
