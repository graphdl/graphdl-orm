// crates/arest-kernel/src/interrupts.rs
//
// Interrupt Descriptor Table setup + hardware IRQ routing.
//
// Breakpoint (#BP, int 3) and double-fault (#DF, int 8) cover the
// CPU-exception path. For device IRQs we remap the legacy 8259 PIC
// so IRQ 0-15 land on vectors 32-47, which keeps them off the
// reserved CPU-exception range 0-31.
//
// Vector layout:
//   0-31     CPU exceptions (IDT entries declared individually;
//            for now only #BP + #DF are populated)
//   32       PIC primary (IRQ 0)    — timer   (unused until #180)
//   33       PIC primary (IRQ 1)    — keyboard  [this commit]
//   34-39    PIC primary (IRQ 2-7)  — unused
//   40       PIC secondary (IRQ 8)  — RTC (masked)
//   41-47    PIC secondary (IRQ 9-15) — unused
//
// The keyboard handler reads a raw scancode from port 0x60, pipes
// it through `pc-keyboard` to get a decoded Unicode character, then
// forwards it to `repl::process_key` for line buffering and dispatch
// (#183). EOI is sent before calling process_key so the PIC is not
// held while dispatch runs.

use crate::gdt::DOUBLE_FAULT_IST_INDEX;
use crate::println;
use pc_keyboard::{DecodedKey, HandleControl, Keyboard, ScancodeSet1, layouts};
use pic8259::ChainedPics;
use spin::{Mutex, Once};
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

/// Base vectors for the two cascaded PICs. Chosen to sit right
/// after the 32 CPU exception slots reserved by the architecture.
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

/// Mapping of hardware IRQ → IDT vector.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Cascaded PIC pair under a spin lock. `ChainedPics::new_contiguous`
/// reserves the 16 vectors starting at PIC_1_OFFSET; `initialize()`
/// issues the ICW sequence that actually performs the remap. Call
/// `PICS.lock().initialize()` once at boot, then unmask individual
/// IRQs as their handlers come online.
pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new_contiguous(PIC_1_OFFSET) });

/// Shared keyboard decoder state — tracks modifiers, shift, caps
/// lock across scancodes. Protected by spin::Mutex because the
/// keyboard handler and (future) any debug printer that wants to
/// query the decoder both take a borrow.
static KEYBOARD: Once<Mutex<Keyboard<layouts::Us104Key, ScancodeSet1>>> = Once::new();

/// IDT instance populated once at boot and left alive for the rest
/// of the kernel's lifetime.
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
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_handler);
        idt
    });
    idt.load();
}

/// Bring hardware IRQs online: remap the PICs, unmask keyboard,
/// then `sti`. Must run after `init_idt` so any IRQ that fires
/// immediately lands in a registered handler.
pub fn init_pic() {
    KEYBOARD.call_once(|| {
        Mutex::new(Keyboard::new(
            ScancodeSet1::new(),
            layouts::Us104Key,
            HandleControl::Ignore,
        ))
    });

    unsafe {
        let mut pics = PICS.lock();
        pics.initialize();
        // Mask everything, then unmask IRQ 1 (keyboard). The timer
        // IRQ 0 comes online in a later commit once we decide how
        // preemption is triggered.
        pics.write_masks(0xFD, 0xFF);
    }
    x86_64::instructions::interrupts::enable();
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

extern "x86-interrupt" fn keyboard_handler(_stack_frame: InterruptStackFrame) {
    let mut port = Port::<u8>::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    // Decode the scancode first so we can determine whether this is
    // an Enter key before deciding when to send EOI.
    let decoded_ch: Option<char> = if let Some(keyboard) = KEYBOARD.get() {
        let mut kb = keyboard.lock();
        if let Ok(Some(event)) = kb.add_byte(scancode) {
            if let Some(key) = kb.process_keyevent(event) {
                match key {
                    DecodedKey::Unicode(ch) => Some(ch),
                    DecodedKey::RawKey(_) => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // PIC EOI: acknowledge the PIC *before* calling dispatch so the
    // keyboard IRQ can fire again while dispatch is printing output.
    // This is safe because we are still in the ISR frame — interrupts
    // are automatically re-enabled by the `iretq` at the end.
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }

    // Forward the decoded character to the REPL. process_key handles
    // buffering, echoing, and (on Enter) dispatch.
    if let Some(ch) = decoded_ch {
        crate::repl::process_key(ch);
    }
}
