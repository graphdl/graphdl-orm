# AREST Kernel — Getting Started

A bare-metal AREST: same engine as the CLI and the Cloudflare Worker, but
shipped as a UEFI binary that boots directly on hardware (or under QEMU).
No Linux underneath. The kernel owns the page table, the IDT, the network
stack (smoltcp), the framebuffer (virtio-gpu / GOP), and the syscall surface.

Three targets:

| Target                          | Where it runs                              |
|---------------------------------|--------------------------------------------|
| `x86_64-unknown-uefi`           | OVMF / QEMU-x86, modern laptops            |
| `aarch64-unknown-uefi`          | AAVMF / QEMU-virt-arm, Raspberry Pi 4      |
| `arest-kernel-armv7-uefi.json`  | ArmVirtPkg / QEMU-virt-arm32, older phones |

The kernel boots into a Slint-rendered launcher. Three apps ship by default:
the unified REPL, an on-screen keyboard, and a HATEOAS resource browser.
Doom, Wine, busybox, and Linux ABI compat are opt-in features.

## Prerequisites

The Docker path needs only Docker. The native path needs:

```bash
# Rust nightly + UEFI targets
rustup toolchain install nightly --profile minimal
rustup component add rust-src --toolchain nightly
rustup target add x86_64-unknown-uefi --toolchain nightly
rustup target add aarch64-unknown-uefi --toolchain nightly

# QEMU + OVMF firmware (host package manager, e.g. on macOS)
brew install qemu
# Linux:    apt install qemu-system-x86 ovmf
# Windows:  choco install qemu  (and grab OVMF.fd manually)
```

## Boot under QEMU (Docker, easiest)

```bash
# x86_64
docker build -t arest-kernel-uefi -f crates/arest-kernel/Dockerfile.uefi .
docker run --rm arest-kernel-uefi

# aarch64
docker build -t arest-kernel-uefi-aarch64 -f crates/arest-kernel/Dockerfile.uefi-aarch64 .
docker run --rm arest-kernel-uefi-aarch64

# armv7
docker build -t arest-kernel-uefi-armv7 -f crates/arest-kernel/Dockerfile.uefi-armv7 .
docker run --rm arest-kernel-uefi-armv7
```

You should see the OVMF boot screen, then the AREST banner, then the
launcher.

## Boot under QEMU (native)

```bash
# Build the .efi
cargo +nightly build --release --target x86_64-unknown-uefi -p arest-kernel
# Output: target/x86_64-unknown-uefi/release/arest-kernel.efi

# Boot interactively (Windows PowerShell helper does the ESP staging)
.\scripts\boot-kernel-uefi.ps1

# Headless smoke test (asserts the boot banner reaches serial)
.\scripts\boot-kernel-uefi.ps1 -Smoke

# aarch64 / armv7 variants
.\scripts\boot-kernel-uefi-aarch64.ps1
.\scripts\boot-kernel-uefi-armv7.ps1
```

On Linux/macOS, run the QEMU command directly — see the helper script body
for the canonical invocation (OVMF code/vars, virtio-net, virtio-gpu,
serial-stdio, ESP loop-mount).

## Boot on real hardware (aarch64 phone)

```bash
# Package the .efi as an Android boot.img
.\scripts\package-aarch64-boot-img.ps1
# Output: arest-kernel-boot.img

# Flash via fastboot (LG Nexus 5/5X/6P, Pixel devices, etc.)
fastboot boot arest-kernel-boot.img       # one-shot: boot without flashing
# OR
fastboot flash boot arest-kernel-boot.img # persistent flash
```

UART-over-USB serial is wired (#392) — attach a serial console to see the
boot banner before the framebuffer comes up.

## Use the checker

The unified REPL renders the current cell. Constraint violations against
that cell are shown inline — same surface the CLI and the worker expose,
but rendered as part of the screen rather than as JSON. Type `verify` at
the REPL to run the full check against the current population.

## Optional features

Each adds a feature flag to the cargo build. Most are off by default for
size or licensing reasons (the default `.efi` is AGPL-3.0-or-later only).

| Feature             | Adds                                                  | Notes                |
|---------------------|-------------------------------------------------------|----------------------|
| `doom`              | Doom WASM launcher app                                | GPL-2.0 binary       |
| `linuxkpi`          | Linux KPI shim (lets unmodified Linux drivers link)   | + GPL-2.0 vendored C |
| `musl-libc`         | Static musl libc.a built against AREST syscalls       | MIT                  |
| `busybox`           | Static busybox (`ls cat echo wc head tail`)           | GPL-2.0 binary       |
| `qt-adapter`        | Qt 6 widgets via linuxkpi loader                       | needs `linuxkpi`     |
| `gtk-adapter`       | GTK 4 widgets via linuxkpi loader                      | needs `linuxkpi`     |
| `compositor-test`   | Checkerboard renderer for foreign-toolkit composer    | dev only             |

```bash
cargo +nightly build --release --target x86_64-unknown-uefi -p arest-kernel \
  --features "doom linuxkpi musl-libc busybox"
```

## Tests

```bash
cargo test --lib -p arest-kernel
```

Inline `#[cfg(test)]` blocks under `process/`, `syscall/`, `synthetic_fs/`,
`composer.rs`, `component_binding.rs`, `toolkit_loop.rs`, `ui_apps/*` run
on the host target. UEFI-only modules are gated out under `cfg(test)`.

## Where next

- `docs/16-uefi-pivot.md` — why UEFI is the only target (BIOS path was deprecated)
- `docs/24-ui-toolkit-decision.md` — why Slint over egui
- `crates/arest-kernel/src/lib.rs` — module tree (each `pub mod` line is a subsystem)
- [`docs/cli.md`](../../docs/cli.md) — same engine on the host
- [`docs/cloud.md`](../../docs/cloud.md) — same engine in the cloud
- [`docs/mcp.md`](../../docs/mcp.md) — agent-facing surface
