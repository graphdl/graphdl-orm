// crates/arest-kernel/src/repl.rs
//
// Kernel REPL — task #183: system_impl over keyboard + console.
//
// Architecture:
//   - `LINE_BUFFER` accumulates printable characters as they arrive
//     from the keyboard ISR. The ISR is kept short: just buffer + echo.
//   - On Enter the ISR calls `process_key('\n')` which takes the
//     accumulated line, dispatches it (still inside the ISR, after
//     the EOI is sent by the caller), and prints a result + fresh
//     prompt. Dispatch is fast for the built-in commands; it is
//     acceptable to do it after EOI so we are no longer holding the
//     PIC hostage.
//   - `dispatch` implements built-in commands and stubs out the
//     arest engine (not yet linked — Cargo.toml has no arest dep).

use alloc::string::{String, ToString};
use spin::Mutex;

/// Maximum line length we accept. Longer lines are silently truncated.
const MAX_LINE: usize = 256;

/// The prompt printed after each command (and at boot).
const PROMPT: &str = "arest> ";

/// Line buffer — accumulates keystrokes between newlines.
/// Wrapped in a spin::Mutex so it is safe to touch from the ISR.
pub static LINE_BUFFER: Mutex<String> = Mutex::new(String::new());

/// Print the REPL prompt. Call once from `kernel_main` after the
/// boot banner, then automatically after every Enter key.
pub fn init() {
    use crate::print;
    print!("{PROMPT}");
}

/// Called from the keyboard ISR for every decoded Unicode character.
///
/// # Interrupt-context constraints
/// This function MUST NOT allocate in a way that can deadlock with
/// another lock held by the interrupted code, and it must return
/// quickly. Printable chars and Backspace are handled inline.
/// Enter triggers `dispatch` but only after the ISR has already
/// sent EOI (the ISR sends EOI before calling `process_key` for
/// Enter — see `interrupts.rs`).
pub fn process_key(ch: char) {
    use crate::print;

    match ch {
        // Enter — take the buffer, dispatch, show result + prompt.
        '\n' | '\r' => {
            // Move the accumulated line out of the buffer.
            let line = {
                let mut buf = LINE_BUFFER.lock();
                let s = buf.clone();
                buf.clear();
                s
            };
            print!("\n");
            let result = dispatch(line.trim());
            if !result.is_empty() {
                print!("{result}\n");
            }
            print!("{PROMPT}");
        }

        // Backspace (ASCII 0x08) or DEL (0x7f).
        '\x08' | '\x7f' => {
            let mut buf = LINE_BUFFER.lock();
            if buf.pop().is_some() {
                // Move cursor back, overwrite with space, move back again.
                print!("\x08 \x08");
            }
        }

        // Printable / typeable character.
        ch if !ch.is_control() => {
            let mut buf = LINE_BUFFER.lock();
            if buf.len() < MAX_LINE {
                buf.push(ch);
                print!("{ch}");
            }
            // Silently discard if buffer is full — do not echo.
        }

        // All other control characters (Escape, Tab, etc.) — ignore.
        _ => {}
    }
}

/// Dispatch a trimmed input line and return a response string.
///
/// Built-in commands are handled directly. The arest engine is not
/// yet linked (no `arest` entry in Cargo.toml) so all other input
/// receives a stub message.
pub fn dispatch(line: &str) -> String {
    match line {
        "" => String::new(),

        "help" => {
            [
                "Built-in commands:",
                "  help  — show this message",
                "  heap  — print allocator stats",
                "  quit  — halt the kernel",
                "",
                "AREST engine not yet linked.",
                "Once the `arest` crate is added to Cargo.toml,",
                "lines will be parsed as AREST expressions.",
            ]
            .join("\n")
        }

        "quit" | "exit" => {
            use crate::println;
            println!("Halting.");
            // Disable interrupts and loop — clean shutdown in QEMU.
            x86_64::instructions::interrupts::disable();
            loop {
                x86_64::instructions::hlt();
            }
        }

        "heap" => {
            // linked_list_allocator::LockedHeap exposes .lock() -> Heap
            // which has .size() (total) and .free() bytes. Only the BIOS
            // path uses it; the UEFI arm rides uefi-rs's pool allocator,
            // which doesn't expose comparable counters — report "n/a".
            #[cfg(not(target_os = "uefi"))]
            {
                let heap = crate::allocator::ALLOCATOR.lock();
                let total = heap.size();
                let free  = heap.free();
                let used  = total - free;
                alloc::format!(
                    "heap: {used} B used / {free} B free / {total} B total"
                )
            }
            #[cfg(target_os = "uefi")]
            {
                alloc::format!(
                    "heap: n/a (uefi-rs pool allocator — counters not exposed)"
                )
            }
        }

        _ => {
            // arest engine not linked yet.
            alloc::format!(
                "unknown command: `{line}`\n\
                 AREST engine not yet linked — add `arest` to Cargo.toml\n\
                 to enable expression evaluation. Type `help` for commands."
            )
        }
    }
}
