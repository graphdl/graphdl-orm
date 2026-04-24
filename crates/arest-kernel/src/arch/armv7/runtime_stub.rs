// crates/arest-kernel/src/arch/armv7/runtime_stub.rs
//
// Minimal `#[global_allocator]` + `#[panic_handler]` stubs for the
// armv7-UEFI scaffold (#346 first commit).
//
// Why this file exists in THIS commit:
//
// `main.rs` declares `extern crate alloc;` unconditionally and is
// `#![no_std]`, which means EVERY target — armv7 included — needs a
// `#[global_allocator]` at link time and a `#[panic_handler]` at
// build time. The aarch64-UEFI arm gets both from `entry_uefi_aarch64.rs`
// (a static `LockedHeap` + a PL011 panic handler), and the x86_64-UEFI
// arm gets both from `entry_uefi.rs`. The armv7 runtime harness
// (`entry_uefi_armv7.rs` — pre/post-EBS banner, ExitBootServices,
// driver bring-up) is deferred to #346d so we can land the target +
// arch scaffold in isolation.
//
// In the meantime, this stub satisfies the linker/checker so
// `cargo +nightly build --target arest-kernel-armv7-uefi.json
// -Z build-std=core,compiler_builtins,alloc` produces a `.efi`
// without needing the rest of the runtime. Both items will be
// REPLACED by `entry_uefi_armv7.rs` when #346d lands — at which
// point this file goes away (or shrinks to a no-op marker module).
//
// Design choices, optimised for "compile-only, never executes":
//
//   * Global allocator: a `BumpStub` that always returns null.
//     No backing storage. Any actual `Box::new` / `Vec::push`
//     reaches `alloc_error_handler` and aborts. That's fine — this
//     scaffold has no runtime entry point so no allocation site is
//     reachable. The aarch64-UEFI `LockedHeap` pattern lands
//     wholesale in #346d.
//
//   * Panic handler: bare `wfi` loop. No PL011 print here — the
//     `arch::armv7::serial::raw_puts` writer would make the panic
//     handler depend on the serial module being intact; since this
//     stub is "should never fire" we keep it self-contained for
//     now. #346d's real panic handler in `entry_uefi_armv7.rs`
//     surfaces via PL011 the same way the aarch64 arm does.
//
//   * `efi_main`: a UEFI-callable entry stub that just calls
//     `halt_forever`. The custom target JSON's
//     `pre-link-args /entry:efi_main` makes this symbol mandatory at
//     link time; the aarch64-UEFI arm gets it from
//     `entry_uefi_aarch64.rs::efi_main` (a real `#[entry]`-decorated
//     function), but until #346d we provide a name-only stub here so
//     the linker resolves it. The stub does not even acknowledge the
//     UEFI argument convention (`ImageHandle` + `*mut SystemTable`)
//     — it's marked `extern "C"` with a unit return so any caller
//     that actually invokes it (firmware) just sees a no-return
//     function. Replaced wholesale in #346d.
//
// Gated behind `cfg(all(target_os = "uefi", target_arch = "arm"))`
// transitively (the parent module already carries that gate via
// `arch/mod.rs`). No additional gate needed here.

use core::alloc::{GlobalAlloc, Layout};

/// Bump-style allocator that always fails. Replaced by the real
/// static-BSS `LockedHeap` in #346d's `entry_uefi_armv7.rs`. Until
/// then, the kernel has no reachable `_start` so no allocation
/// site executes — `alloc()` returning null is the correct stub
/// behavior.
struct BumpStub;

unsafe impl GlobalAlloc for BumpStub {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        // No backing storage in the scaffold. Any caller that gets
        // here is wrong — the real allocator lands in #346d.
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // `alloc` always returns null, so any pointer passed here
        // is invalid. Nothing to do.
    }
}

#[global_allocator]
static ALLOC_STUB: BumpStub = BumpStub;

/// Compile-only UEFI entry stub. The target JSON's
/// `/entry:efi_main` makes this symbol mandatory at link time; the
/// real `#[entry]`-decorated `efi_main` lands with the runtime
/// harness in #346d (mirroring `entry_uefi_aarch64.rs::efi_main`).
///
/// Until then this stub immediately diverges into a `wfi` halt loop
/// so that even if the firmware ever does invoke it (during a
/// `qemu-system-arm` smoke), the CPU parks instead of falling off
/// the end of the function. We don't pretend to follow the UEFI
/// `efi_main(ImageHandle, *mut SystemTable) -> Status` calling
/// convention here — the arguments arrive in `r0`/`r1` and we simply
/// ignore them. The `#[unsafe(no_mangle)]` ensures the symbol name
/// matches `/entry:efi_main` literally so the firmware can locate
/// it via the PE32+ entry-point header.
///
/// Replaced wholesale by `entry_uefi_armv7.rs` in #346d.
#[unsafe(no_mangle)]
pub extern "C" fn efi_main() -> ! {
    crate::arch::armv7::halt_forever()
}

/// Compile-only panic handler for the scaffold. `entry_uefi_armv7.rs`
/// (#346d) will replace this with a PL011-printing handler shaped
/// like `entry_uefi_aarch64.rs`'s.
///
/// The bare `wfi` loop matches `arch::armv7::halt_forever` — same
/// instruction, just inlined here so the handler doesn't pull in
/// any other module. If the serial module itself caused the panic,
/// this stays alive; the firmware can still inspect the halted
/// CPU state.
#[panic_handler]
fn panic_stub(_info: &core::panic::PanicInfo) -> ! {
    loop {
        // SAFETY: `wfi` is unprivileged in PL1 on armv7 and has no
        // side effects beyond pausing until the next interrupt.
        // Same opts the public `halt_forever` uses.
        unsafe {
            core::arch::asm!(
                "wfi",
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}
