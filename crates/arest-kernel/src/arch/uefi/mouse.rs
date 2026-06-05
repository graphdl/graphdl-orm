// crates/arest-kernel/src/arch/uefi/mouse.rs
//
// PS/2 mouse hardware driver for the UEFI x86_64 path (#598 pointer
// producer). Sibling of `keyboard.rs`: the 8042's aux device streams
// 3-byte packets on IRQ 12 (slave PIC). This module owns the hardware —
// the 8042 aux bring-up, the per-byte accumulator state, and the
// translation of a completed packet into `pointer::PointerEvent`
// pushes. The pure parse / assembly / button-edge logic lives in
// `crate::ps2_mouse` (host-tested); this file is boot-verified.
//
// Gated on `feature = "slint"`: the pointer ring this feeds is drained
// only by the slint launcher's per-frame dispatch. Headless
// `--no-default-features --features server` builds elide the module,
// leave IRQ 12 masked, and keep IDT vector 44 at the EOI-only stub.

use spin::Mutex;
use x86_64::instructions::port::Port;

use super::pointer::{self, PointerEvent};
use crate::ps2_mouse::{Accumulator, MouseFrame};

// Linux `BTN_*` input codes — match the `PointerEvent::Button` docs in
// `pointer.rs` so the launcher's BTN_LEFT-vs-BTN_RIGHT dispatch reads
// them correctly.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Packet assembler + button-edge tracker. Single-writer (the IRQ 12
/// handler). `Mutex` rather than a bare cell so a future peeker can't
/// trip the borrow checker against the ISR's `feed`.
static ACC: Mutex<Accumulator> = Mutex::new(Accumulator::new());

/// Feed one aux byte from the IRQ 12 handler. Assembles it into the
/// pending packet; on a completed packet, translates the resulting
/// `MouseFrame` into `pointer::PointerEvent` pushes — a `RelMove` when
/// there's motion, a `Button` per button edge, then a `Sync` to close
/// the frame. This is the exact shape the linuxkpi virtio-input path
/// emits, so the launcher's existing pointer drain handles both
/// producers identically.
pub fn handle_aux_byte(byte: u8) {
    // Resolve the frame under the ACC lock, then release it before
    // pushing — `pointer::push` takes the ring lock (and logs), so the
    // ACC hold-time stays bounded to the assembly step.
    let frame: Option<MouseFrame> = ACC.lock().feed(byte);
    let Some(frame) = frame else { return };

    if frame.dx != 0 || frame.dy != 0 {
        pointer::push_pointer_event(PointerEvent::RelMove { dx: frame.dx, dy: frame.dy });
    }
    if let Some(pressed) = frame.left_edge {
        pointer::push_pointer_event(PointerEvent::Button { button: BTN_LEFT, pressed });
    }
    if let Some(pressed) = frame.right_edge {
        pointer::push_pointer_event(PointerEvent::Button { button: BTN_RIGHT, pressed });
    }
    if let Some(pressed) = frame.middle_edge {
        pointer::push_pointer_event(PointerEvent::Button { button: BTN_MIDDLE, pressed });
    }
    pointer::push_pointer_event(PointerEvent::Sync);
}

// ── 8042 aux (mouse) bring-up ───────────────────────────────────────
//
// The legacy 8042 controller exposes the mouse as its "second PS/2
// port" (aux). Bring-up: enable the aux port (0xA8), set the
// aux-IRQ-enable bit in the controller config byte, then reset + set
// defaults + enable data reporting on the mouse device itself (each
// command addressed to the aux device via the 0xD4 write prefix and
// ACK'd with 0xFA). Every poll is bounded so a controller without an
// aux device (a future keyboard-only bare-metal box) can't wedge boot;
// QEMU always provides the device, so the happy path always completes.

/// Controller status bit: output buffer full (a byte waits in 0x60).
const STATUS_OUTPUT_FULL: u8 = 0x01;
/// Controller status bit: input buffer full (a prior 0x60/0x64 write
/// hasn't been consumed yet).
const STATUS_INPUT_FULL: u8 = 0x02;
/// Upper bound on each status poll. ~100k port reads is microseconds on
/// real silicon and a few ms under QEMU TCG — long enough for the
/// controller to respond, short enough that an absent device fails fast
/// instead of hanging the boot.
const POLL_LIMIT: u32 = 100_000;

/// The two 8042 ports: 0x60 (data) and 0x64 (write = command, read =
/// status).
struct Ports {
    data: Port<u8>,
    cmd: Port<u8>,
}

impl Ports {
    fn new() -> Self {
        Self { data: Port::new(0x60), cmd: Port::new(0x64) }
    }

    /// Bounded-spin until the input buffer drains, then write a command
    /// byte to 0x64.
    fn write_cmd(&mut self, b: u8) {
        self.wait_input_clear();
        // SAFETY: 0x64 is the 8042 command port; writing a documented
        // controller command is the standard control path.
        unsafe { self.cmd.write(b) };
    }

    /// Bounded-spin until the input buffer drains, then write a data
    /// byte to 0x60.
    fn write_data(&mut self, b: u8) {
        self.wait_input_clear();
        // SAFETY: 0x60 is the 8042 data port; writing after the input
        // buffer is clear is the documented data-write handshake.
        unsafe { self.data.write(b) };
    }

    /// Bounded-spin for an output byte, then read it from 0x60. `None`
    /// if nothing arrived within the poll budget.
    fn read_data(&mut self) -> Option<u8> {
        for _ in 0..POLL_LIMIT {
            // SAFETY: reading 0x64 returns the controller status byte
            // with no side effects.
            let status = unsafe { self.cmd.read() };
            if status & STATUS_OUTPUT_FULL != 0 {
                // SAFETY: output-buffer-full is set, so 0x60 holds a
                // valid byte; reading it clears the buffer.
                return Some(unsafe { self.data.read() });
            }
        }
        None
    }

    /// Bounded-spin until the controller's input buffer is empty.
    fn wait_input_clear(&mut self) {
        for _ in 0..POLL_LIMIT {
            // SAFETY: status read, no side effects.
            let status = unsafe { self.cmd.read() };
            if status & STATUS_INPUT_FULL == 0 {
                return;
            }
        }
    }

    /// Send one command to the aux (mouse) device. The 0xD4 prefix
    /// routes the following data byte to the mouse rather than the
    /// keyboard. Returns the device's ACK byte (0xFA on success).
    fn mouse_cmd(&mut self, b: u8) -> Option<u8> {
        self.write_cmd(0xD4);
        self.write_data(b);
        self.read_data()
    }
}

/// Bring the PS/2 aux (mouse) device online. Called from
/// `interrupts::pic_init` before IRQ 12 is unmasked, so the device is
/// configured and streaming by the time the line opens.
pub fn init() {
    let mut p = Ports::new();

    // Enable the aux device (second PS/2 port).
    p.write_cmd(0xA8);

    // Read the controller config byte, set the aux-IRQ-enable bit
    // (bit 1), clear the aux-clock-disable bit (bit 5; 0 = clock on),
    // and write it back.
    p.write_cmd(0x20);
    if let Some(mut config) = p.read_data() {
        config |= 0x02;
        config &= !0x20;
        p.write_cmd(0x60);
        p.write_data(config);
    }

    // Reset the mouse (0xFF): it ACKs (0xFA), then streams a self-test
    // result (0xAA) + device id (0x00) — drain both.
    let _ = p.mouse_cmd(0xFF);
    let _ = p.read_data();
    let _ = p.read_data();

    // Set defaults (0xF6), then enable data reporting (0xF4) so the
    // device starts delivering movement packets on IRQ 12.
    let _ = p.mouse_cmd(0xF6);
    let _ = p.mouse_cmd(0xF4);
}
