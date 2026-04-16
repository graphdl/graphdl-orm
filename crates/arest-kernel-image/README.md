# arest-kernel-image

Builds a bootable BIOS disk image from an `arest-kernel` ELF using
the rust-osdev `bootloader` crate (0.11).

## Usage

```bash
cd crates/arest-kernel
cargo build --target x86_64-unknown-none

cd ../arest-kernel-image
cargo run -- \
    ../arest-kernel/target/x86_64-unknown-none/debug/arest-kernel \
    ../../target/arest-kernel.bios.img

qemu-system-x86_64 \
    -drive format=raw,file=../../target/arest-kernel.bios.img \
    -serial stdio -no-reboot -no-shutdown
```

## Host requirements

- Rust **nightly** — pinned via `rust-toolchain.toml` in this crate.
  `bootloader` 0.11's `build.rs` compiles its BIOS and UEFI stage
  binaries with `-Z build-std`, which is nightly-only.
- `rust-src` component — `rustup component add rust-src --toolchain nightly`.

## Known issue: Windows MSVC host

`bootloader-x86_64-uefi` and `bootloader-x86_64-bios-stage-3` crash
with `STATUS_STACK_BUFFER_OVERRUN` (0xc0000409) when their build.rs
invokes `cargo install` on Windows MSVC. This is upstream — tracked
in the rust-osdev issue tracker. Work around by building under
**WSL**, macOS, or Linux:

```bash
# from Linux / WSL
cargo run -p arest-kernel-image -- path/to/arest-kernel out.img
```

The `arest-kernel` crate itself builds cleanly on Windows MSVC — the
bare-metal ELF target (`x86_64-unknown-none`) is independent of host
OS. Only the image-concatenation step (this crate) needs a working
bootloader build, which is what the Windows issue blocks today.

Once the upstream fix lands we can drop this caveat and rely on the
single-command flow everywhere.
