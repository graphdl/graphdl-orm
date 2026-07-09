// engine/os — AREST 0.9.0 on bare UEFI: the entry shim and the boot
// banner. The firmware probes the PE32+ AddressOfEntryPoint for the
// symbol the uefi::entry macro exports; no_main tells rustc not to
// expect a Rust main (the same shape the pre-0.9.0 kernel, redox, and
// the rust-osdev book use). ConOut is mirrored to COM1 by OVMF, so the
// QEMU harness asserts the banner from the serial capture.
//
// The server target's arc from here: canon at boot (the store sidecar
// baked into the image), the verb table over virtio-net — the engine
// surface, headless. mini adds a text console; full realizes the canon
// view trees through Slint on the framebuffer.
#![no_std]
#![no_main]

extern crate alloc;

use uefi::prelude::*;

#[entry]
fn efi_main() -> Status {
    uefi::helpers::init().expect("uefi helpers");

    log::info!("AREST OS 0.9.0");
    log::info!("target: {}", TARGET);
    log::info!("engine: canon-at-boot pending; verb table pending");
    log::info!("boot: complete");

    // The engine loop replaces this idle: the server target parks on
    // the network poll once virtio-net lands.
    loop {
        uefi::boot::stall(1_000_000);
    }
}

#[cfg(feature = "full")]
const TARGET: &str = "full";
#[cfg(all(feature = "mini", not(feature = "full")))]
const TARGET: &str = "mini";
#[cfg(all(feature = "server", not(feature = "mini")))]
const TARGET: &str = "server";
