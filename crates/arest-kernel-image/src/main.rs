// crates/arest-kernel-image/src/main.rs
//
// Image-builder companion to the arest-kernel crate. Takes a kernel
// ELF path and an output path and writes a bootable BIOS disk image
// that GRUB / QEMU / real hardware can boot.
//
// Kept as a separate crate (and not a build.rs inside arest-kernel)
// because the kernel crate builds for `x86_64-unknown-none` — it has
// no host OS, no `std`, no filesystem. The image-builder is an
// ordinary host-side program that reads the kernel ELF off disk and
// writes a concatenated disk image next to it.
//
// The underlying `bootloader::DiskImageBuilder` API handles the
// boot-sector stub, the second-stage loader that parses the ELF,
// the trampoline into 64-bit long mode, and the BootInfo hand-off.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let kernel = args
        .next()
        .map(PathBuf::from)
        .expect("usage: arest-kernel-image <kernel-elf> <output.img>");
    let output = args
        .next()
        .map(PathBuf::from)
        .expect("usage: arest-kernel-image <kernel-elf> <output.img>");

    if !kernel.is_file() {
        eprintln!("kernel ELF not found: {}", kernel.display());
        std::process::exit(2);
    }

    bootloader::DiskImageBuilder::new(kernel.clone())
        .create_bios_image(&output)
        .unwrap_or_else(|e| {
            eprintln!("failed to build BIOS image: {e}");
            std::process::exit(1);
        });

    println!("built {}", output.display());
}
