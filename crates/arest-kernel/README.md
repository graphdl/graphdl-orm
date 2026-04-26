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
docker run --rm -p 8080:8080 arest-kernel-uefi

# aarch64
docker build -t arest-kernel-uefi-aarch64 -f crates/arest-kernel/Dockerfile.uefi-aarch64 .
docker run --rm -p 8080:8080 arest-kernel-uefi-aarch64

# armv7
docker build -t arest-kernel-uefi-armv7 -f crates/arest-kernel/Dockerfile.uefi-armv7 .
docker run --rm -p 8080:8080 arest-kernel-uefi-armv7
```

The container streams the OVMF boot screen, then the AREST kernel
banner, on stdout. The final line you should see is `ui: launcher
running` — that confirms the kernel ran through ExitBootServices, set
up the IDT / heap / virtio devices / smoltcp / HTTP listener, and
yielded to the Slint event loop.

**Headless by design.** The Dockerfile launches QEMU with
`-display none`, so the Slint launcher / unified REPL / on-screen
keyboard render to a virtio-gpu framebuffer no one is looking at.
The Slint UI walkthrough in the next section describes what the
kernel *supports*; to actually see it you need either a Dockerfile
variant with `-display gtk` + X11 forwarding, or run QEMU natively.

Two interaction surfaces work in headless mode out of the box:

1. **Serial console** — boot output and any kernel `println!` lands
   on container stdout (and on a future serial-input REPL, when wired).
2. **HTTP API on `localhost:8080`** — the kernel registers
   `arest_http_handler` on guest port 80; the Dockerfile forwards
   container 8080 → guest 80 via QEMU SLiRP. Same verb surface as the
   CLI and the worker, served by the in-kernel `arest_http_handler`
   that combines `assets::lookup` (baked ui.do bundle) with
   `system::dispatch` (engine ρ).

## Persisting readings across container restarts

The Dockerfile creates a 1 MiB raw image at `/disk.img` inside the
container and exposes it as a virtio-blk device (`#335` storage,
`#560` LoadReading persistence). Without a volume mount this image
lives only in the container layer and `docker run --rm` wipes it on
exit — readings load fine *during* a session but evaporate on next
boot.

Bind-mount a host file at `/disk.img` to keep the ring across runs:

```bash
# One-time: create the empty backing image
dd if=/dev/zero of=arest-disk.img bs=1M count=1

# Boot with the host file as the virtio-blk backing
docker run --rm \
    -p 8080:8080 \
    -v "$(pwd)/arest-disk.img:/disk.img" \
    arest-kernel-uefi
```

Windows PowerShell:

```powershell
fsutil file createnew arest-disk.img 1048576
docker run --rm -p 8080:8080 -v "${PWD}/arest-disk.img:/disk.img" arest-kernel-uefi
```

LoadReading bodies written this session replay on every subsequent
boot via `load_reading_persist::replay_on_boot` — provided the
`arest-disk.img` host file is mounted at the same path each time.

## Quickstart you can run today (Docker headless + curl)

While the kernel is booted in one terminal, drive it from another via
the HTTP surface:

```bash
# Schema introspection — every endpoint the engine generated from
# the bundled readings (core / ui / os / templates per the kernel's
# default feature set).
curl http://localhost:8080/api/openapi.json | jq

# List entities of a noun (empty on first boot)
curl http://localhost:8080/arest/default/Organization

# Create one
curl -X POST http://localhost:8080/arest/default/Organization \
    -H "Content-Type: application/json" \
    -d '{"id":"acme","name":"Acme Corp"}'

# Load a reading at runtime — persists to /disk.img if you mounted
# the host file above; replays on next boot.
curl -X POST http://localhost:8080/arest/default/load_reading \
    -H "Content-Type: application/json" \
    -d '{"name":"my-orders","body":"## Entity Types\nOrder(.Order Id) is an entity type.\nCustomer(.Name) is an entity type.\n## Fact Types\nOrder was placed by Customer.\n  Each Order was placed by exactly one Customer.\n"}'

# Now exercise the freshly-loaded reading
curl -X POST http://localhost:8080/arest/default/Order \
    -H "Content-Type: application/json" \
    -d '{"id":"ord-1","customer":"acme"}'
curl http://localhost:8080/arest/default/Order/ord-1
```

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

## Using the OS

When boot completes you land on the launcher with three default apps:
**Unified REPL**, **On-screen Keyboard**, and **HATEOAS Browser**. Tap or
click an icon to open one. With a touchscreen the on-screen keyboard
appears automatically when a text input gains focus — touch-mode detection
(#466) flips the whole OS into spacious DensityScale.

### The Unified REPL

The REPL is the primary interaction surface. It renders the *current
cell* — every screen IS a cell from the compiled state — with its
surrounding cells linked HATEOAS-style. An action panel shows every legal
next step: state-machine transitions per Theorem 5 plus SYSTEM verbs that
apply to the current cell. A breadcrumb tracks where you've been.

Two ways to issue a SYSTEM call:

1. **Click an action button.** If the current cell is an `Order` in
   status `In Cart`, the action panel surfaces a `place` button. Click
   to fire the transition.
2. **Type the verb at the prompt** — same `<key> <input>` shape as the
   CLI:

   ```
   create:Order <<Order Id, ord-1>, <Customer, acme>>
   transition:Order <ord-1, place>
   get:Order ord-1
   list:Order
   query:Order_was_placed_by_Customer {"Customer": "acme"}
   ```

The result becomes a new screen. The breadcrumb extends; back navigates.

### Navigating the cell graph

Every link on a screen is a HATEOAS pointer to another cell. Click to
follow it. There is no URL bar — the cell graph IS the address space.

### Loading a new reading at runtime

`load_reading` is a SYSTEM verb on any domain screen:

```
load_reading:my-app <reading body, or paste from a File cell>
```

The kernel parses it, runs the deontic gate, registers the new fact
types and constraints, and persists the body to virtio-blk so it
replays on next boot (#555 / #560). No reflash, no reboot.

### Reading constraint violations

A mutation that violates a constraint returns `Violation` cells in the
response (Theorem 4 — violations are first-class facts, not exceptions).
The REPL renders them alongside the result, so the bug has a coordinate
on screen. Click a violation to navigate to the rule that fired.

### Running the checker on the whole population

```
verify
```

Runs the full deontic + alethic check against every cell in P. Output
is a list of `Violation` cells — click any one to drill in.

### Browsing without typing

The standalone **HATEOAS Browser** app is the same cell graph as the
REPL but read-only — useful when you want to navigate without
accidentally firing a SYSTEM verb. Same breadcrumb + back/forward.

### Inspecting the kernel itself

The kernel projects its own state through the synthetic filesystem. From
the REPL:

```
get:File /proc/cpuinfo
get:File /proc/meminfo
get:File /proc/self/status
get:File /proc/self/cmdline
get:File /proc/self/maps
list:File /proc        # walk every per-pid projection
```

Each `/proc/*` path is a synthesized File cell rendered from live system
cells (#534 / #535) — not a stored byte-stream. Editing one is a no-op;
the projection re-runs on every read.

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
