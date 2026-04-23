# 16. UEFI Pivot — One Kernel, Many Architectures

## Why

The kernel as shipped (#174–#184) targets `x86_64-unknown-none` and
leans on BIOS-era expedients: `bootloader_api 0.11` for the boot
handoff, `uart_16550` for serial, `pic8259` for interrupts, the
`x86_64` crate for IDT/GDT/paging. None of that was architectural
necessity — the kernel is written against specific silicon because
it was the fastest path to an x86 smoke. The UEFI specification is
exactly the hardware-abstraction layer this class of kernel wants:
firmware-provided serial, memory map, framebuffer, and device
discovery on both x86_64 and aarch64 at the same API, shipping on
laptops, ARM servers, Raspberry Pi 4, QEMU-virt, and a growing set
of phones (Pixel GBL, Project Mainline).

Re-targeting against UEFI lets one kernel source tree boot on
x86_64 and aarch64 with no arch-specific code above the ExitBoot­
Services handoff. Arch-specific bits survive — GIC vs APIC for
interrupts, TTBR vs CR3 for paging, vector tables vs IDT — but
they live below a thin `arch/` trait that covers three methods,
not thirty.

## Target set

| Target                         | Firmware       | Used by                        |
|--------------------------------|----------------|--------------------------------|
| `x86_64-unknown-uefi`          | OVMF           | QEMU-x86, laptops, Pixel GBL   |
| `aarch64-unknown-uefi`         | AAVMF          | QEMU-virt arm, Raspberry Pi 4  |
| `aarch64-unknown-none`         | Android fastboot + DT | real phones (#347) |
| `armv7a-none-eabihf`           | Android fastboot + DT | older 32-bit phones (#346) |

UEFI covers the top two. The bottom two (fastboot + device tree)
are how Android phones actually boot; they share the post-entry
kernel but take a different entry path. Both paths converge at a
shared `kernel_run()` after firmware / bootloader handoff.

## Kernel structure

```
crates/arest-kernel/src/
  entry/
    uefi.rs           — #[cfg(target_os = "uefi")] efi_main
    bios.rs           — #[cfg(target_os = "none", target_arch = "x86_64")] bootloader_api entry (legacy)
    fastboot.rs       — #[cfg(target_os = "none", target_arch ∈ {aarch64, arm})] raw DT entry (#347)
  arch/
    mod.rs            — trait Arch: boot_setup / init_interrupts / map_page / ...
    x86_64.rs         — APIC, IDT, CR3, MSR syscall
    aarch64.rs        — GIC, vector table, TTBR0/1, SVC syscall
    armv7.rs          — GIC, short-descriptor PT, SWI syscall
  kernel.rs           — kernel_run(BootInfo) — arch-neutral after handoff
  { serial, block, net, http, assets, system, block_storage } — reused unchanged
```

Each `entry/*.rs` builds a shared `BootInfo { memory_regions,
framebuffer, device_tree?, command_line }` and hands off to
`kernel::kernel_run`. Arch-specific code calls through the `Arch`
trait; everything above the trait is one copy.

## UEFI-specific plumbing

1. **Entry.** `uefi-rs` provides an `#[entry]` macro and
   `SystemTable<Boot>`.
2. **Serial pre-ExitBootServices.** UEFI `ConOut` (Simple Text
   Output Protocol). `println!` routes there until ExitBootServices.
3. **Memory map.** `BootServices::memory_map` → `BootInfo.memory_regions`.
   Same shape as `bootloader_api`'s version, so `memory::init`
   needs no change at the signature level.
4. **ExitBootServices.** After calling, firmware services become
   invalid. From here the kernel drives real hardware — serial
   switches to PL011 (aarch64) or COM1 (x86_64), interrupts go
   through the arch-specific GIC/APIC setup.
5. **Framebuffer.** `GraphicsOutputProtocol` → base pointer + pitch.
   Handed to `BootInfo`. Used by future #269/#270/#271 (Doom
   framebuffer) without a virtio-gpu driver.

## Staging

The pivot is incremental — every step keeps the existing x86_64
BIOS path bootable via `entry/bios.rs`, until the UEFI path reaches
feature parity. Then BIOS entry gets retired.

1. **Scaffold.** Add `uefi-rs` dep (gated on `target_os = "uefi"`).
   Add `entry/uefi.rs` with an `efi_main` that prints "AREST kernel
   UEFI scaffold" and halts. Add `x86_64-unknown-uefi` to
   `rust-toolchain.toml` targets. Build succeeds; boot under OVMF
   prints the banner. BIOS path unchanged.
2. **Extract `arch` trait.** Move x86-specific code behind
   `arch::Arch::*`. x86_64 impl is thin (just wraps what
   `interrupts.rs` / `gdt.rs` already do). No behavioural change.
3. **Route `println!` through firmware then arch.** UEFI boot
   prints via ConOut pre-ExitBootServices, then through a serial
   driver the arch impl provides. BIOS path's 16550 driver becomes
   the x86_64 `arch::serial` impl.
4. **ExitBootServices + hand-off to `kernel_run`.** UEFI path now
   reaches the same state as the BIOS path. Run a parallel E2E
   under both.
5. **Add `aarch64-unknown-uefi`.** Implement
   `arch::aarch64` with GIC, PL011, TTBR. Boot under QEMU-virt +
   AAVMF. E2E.
6. **Retire BIOS path.** `entry/bios.rs` deleted; `bootloader_api`,
   `uart_16550`, `pic8259`, `pc-keyboard` follow. Kernel is UEFI-only
   and runs on both arches from one build tree.
7. **Add `aarch64-unknown-none` fastboot entry (#347).** Reuses
   `arch::aarch64` entirely; only the entry differs (raw DT pointer
   in x0).
8. **Add `armv7a-none-eabihf` (#346).** Separate `arch::armv7`;
   same pattern.

## What ships in this commit

The design only. Code scaffolding (step 1: `uefi-rs` dep + stub
`entry/uefi.rs` + `x86_64-unknown-uefi` toolchain target) follows
in the next commit under #344; each subsequent stage tracks as its
own commit under #344 / #345 / #346 / #347.
