# arest-kernel-image: Android boot.img wrapper (#390)

Packages the aarch64-unknown-uefi build of `arest-kernel` into an
Android boot image (`boot.img`) so it can eventually be flashed
with `fastboot boot` on a device whose bootloader speaks the
legacy AOSP boot image format.

Target device: **Nexus 5X** (LG Bullhead, msm8992 / Snapdragon 808).
Chosen because its `device/lge/bullhead/BoardConfig.mk` has wide
AOSP coverage and the offsets are well-documented. Nexus 5
(hammerhead) and Nexus 6P (angler) are follow-ups in #393.

## Build

```powershell
.\scripts\package-aarch64-boot-img.ps1
```

Output: `target/boot.img`. The script verifies the artifact starts
with the `ANDROID!` magic before exiting 0.

The Docker image is built from `Dockerfile.boot-img` in this
directory. It reuses the same Rust nightly toolchain layer as
`Dockerfile.uefi-aarch64`, so the kernel build artifacts cache
across both Dockerfiles on the same host.

## Pipeline

1. **Stage 1 (builder).** Mirrors `Dockerfile.uefi-aarch64`.
   Produces `target/aarch64-unknown-uefi/release/arest-kernel.efi`,
   a PE32+ executable with the UEFI entry point in `_start`.
2. **Stage 2 (packager).** Strips the PE wrapper with
   `llvm-objcopy -O binary` to a flat `arest-kernel.img`, then
   runs AOSP `mkbootimg` (installed from PyPI as `mkbootimg`) to
   produce `/boot.img` with Nexus 5X offsets.
3. **Verification.** A `RUN` step inside the Docker image asserts
   `/boot.img` is non-zero and starts with `ANDROID!`. The
   PowerShell wrapper repeats the check on the host after `docker
   cp` to catch transport regressions.

## Bullhead boot image parameters

Copied verbatim from AOSP's `device/lge/bullhead/BoardConfig.mk`:

| `mkbootimg` flag    | Value          | Source                  |
|---------------------|----------------|-------------------------|
| `--base`            | `0x00000000`   | `BOARD_KERNEL_BASE`     |
| `--kernel_offset`   | `0x00008000`   | `BOARD_KERNEL_OFFSET`   |
| `--ramdisk_offset`  | `0x02000000`   | `BOARD_RAMDISK_OFFSET`  |
| `--tags_offset`     | `0x01e00000`   | `BOARD_TAGS_OFFSET`     |
| `--pagesize`        | `4096`         | `BOARD_KERNEL_PAGESIZE` |
| `--header_version`  | `0`            | legacy / pre-Pixel-3    |
| `--cmdline`         | `console=ttyMSM0,115200,n8 androidboot.console=ttyMSM0` | trimmed boot cmdline |
| `--ramdisk`         | `/dev/null`    | (no initramfs needed)   |

`mkbootimg` requires *some* `--ramdisk` argument on header v0 and
treats `/dev/null` as a zero-length ramdisk; the bootloader then
treats the missing initramfs as "boot the kernel directly", which
is what we want for a single-binary AREST kernel.

## Known limitation: no ARM64 boot header

**The produced `boot.img` is a packaging-only scaffold. It will
not boot on a real Nexus 5X yet.**

Why: Android's bootloader on Nexus 5X expects the kernel slot
inside the boot image to begin with an **ARM64 boot protocol**
header (see `Documentation/arm64/booting.rst` in the Linux kernel
tree). The first 8 bytes are a relative branch to the entry, the
next 8 are the text offset, and the magic `ARM\x64` (`0x644d5241`,
little-endian) lives at offset `0x38`. The bootloader checks the
magic before relocating the kernel; without it, the device
fastfails with "missing arm64 magic" (or just hangs in the
bootloader, depending on revision).

The current `arest-kernel` only carries a PE32+ header (the EFI
entry point goes through uefi-rs's `_start`). When `llvm-objcopy
-O binary` strips the PE wrapper, the entry-point metadata is
discarded, but no ARM64 boot header is grafted on -- the flat
binary's offset 0 is just whatever section LLD placed first
(typically the start of `.text`).

Resolving this is a kernel-source change, not a packaging change,
and is therefore out of scope for #390. The follow-up needs to:

1. Grow an `entry_arm64_boot.S` (or similar) carrying the 64-byte
   ARM64 boot header, and place it at offset 0 of the kernel
   binary via a linker script section ordering hint.
2. Make the header's branch target the existing aarch64 entry
   path (after the equivalent of the UEFI bring-up that's
   currently bundled into the `aarch64-unknown-uefi` start).
3. Ship a `aarch64-unknown-none` build (or comparable) so the
   linker doesn't insert PE32+ headers ahead of our ARM64 magic.

Until then, this boot.img is useful for:

- Wiring up the build pipeline (this commit).
- Verifying `mkbootimg` accepts our offsets.
- Smoke-testing the host-side extraction + ANDROID! magic check.
- Comparing header layouts vs. a known-good Nexus 5X boot image
  (e.g., `unpack_bootimg --boot_img boot.img`).

It is **not** safe to flash to a real device with `fastboot flash
boot` -- doing so leaves the device unbootable until the previous
boot image is restored. `fastboot boot boot.img` (one-shot RAM
boot, no flash) is also not expected to succeed today, but it
won't brick the device because nothing is written to flash.

## File ownership (for the staging-discipline gate in CLAUDE.md)

This task (#390) owns:

- `crates/arest-kernel-image/Dockerfile.boot-img` (new)
- `crates/arest-kernel-image/boot-img-README.md` (this file, new)
- `scripts/package-aarch64-boot-img.ps1` (new)

It does **not** touch:

- The aarch64 kernel source under `crates/arest-kernel/src/arch/`.
- `Dockerfile.uefi*` or `Dockerfile` -- existing pipelines stay
  untouched.
- `boot-kernel-uefi*.ps1` -- existing harness stays untouched.

The kernel-source ARM64 boot header work blocks actual booting and
will be a separate task.
