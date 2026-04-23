# crates/arest-kernel-image/Dockerfile
#
# Builds the arest-kernel ELF, wraps it in a bootable BIOS disk image
# via the `bootloader` crate, and (optionally) boots it under QEMU so
# the serial output streams to stdout.
#
# Usage from repo root:
#   docker build -t arest-kernel -f crates/arest-kernel-image/Dockerfile .
#   docker run --rm arest-kernel
#
# The `docker run` step launches QEMU inside the container. Serial
# output (COM1 @ 0x3F8) is wired to stdout via `-serial stdio`. The
# kernel prints its banner, proves the allocator and IDT, echoes
# keyboard input (no keyboard in headless mode, so it halts after the
# banner), and QEMU exits on triple-fault or after 5s timeout.

FROM rust:latest AS builder

# Install nightly + components the kernel and image-builder need.
# `llvm-tools-preview` provides the llvm-objcopy / llvm-ar binaries
# that bootloader 0.11's build.rs expects when splicing BIOS stages
# with the kernel ELF — without it the build panics on
# "failed to get llvm tools: NotFound".
RUN rustup toolchain install nightly --profile minimal \
    && rustup component add rust-src --toolchain nightly \
    && rustup component add llvm-tools-preview --toolchain nightly \
    && rustup target add x86_64-unknown-none --toolchain nightly

WORKDIR /build

# Copy kernel crate
COPY crates/arest-kernel/Cargo.toml crates/arest-kernel/Cargo.toml
COPY crates/arest-kernel/build.rs crates/arest-kernel/build.rs
COPY crates/arest-kernel/src/ crates/arest-kernel/src/
COPY crates/arest-kernel/rust-toolchain.toml crates/arest-kernel/rust-toolchain.toml

# Copy the ui.do static bundle (#266). `build.rs` bakes every file
# under `apps/ui.do/dist/` into the kernel ELF via `include_bytes!`,
# so the handler can serve `/` and `/assets/*` without a filesystem.
# If the bundle is missing, `build.rs` emits an empty asset table
# and the kernel falls back to the plaintext welcome banner.
COPY apps/ui.do/dist/ apps/ui.do/dist/

# Copy arest engine crate — the kernel links it in no_std mode (#265)
# so the Docker build needs the sources plus every reading the
# metamodel include_str! pulls in.
COPY crates/arest/Cargo.toml crates/arest/Cargo.toml
COPY crates/arest/src/ crates/arest/src/
COPY readings/ readings/

# Copy image-builder crate
COPY crates/arest-kernel-image/Cargo.toml crates/arest-kernel-image/Cargo.toml
COPY crates/arest-kernel-image/src/ crates/arest-kernel-image/src/
COPY crates/arest-kernel-image/rust-toolchain.toml crates/arest-kernel-image/rust-toolchain.toml

# Build kernel ELF (nightly, bare-metal target)
RUN cd crates/arest-kernel \
    && cargo +nightly build --target x86_64-unknown-none --release

# Build image-builder (nightly, host target — needs -Z build-std for bootloader stages)
RUN cd crates/arest-kernel-image \
    && cargo +nightly build --release

# Produce the bootable disk image
RUN crates/arest-kernel-image/target/release/arest-kernel-image \
    crates/arest-kernel/target/x86_64-unknown-none/release/arest-kernel \
    /build/arest-kernel.bios.img

# ── Runtime stage: QEMU only ────────────────────────────────────────
FROM debian:bookworm-slim AS runner

RUN apt-get update && apt-get install -y --no-install-recommends \
    qemu-system-x86 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/arest-kernel.bios.img /kernel.img

# Pre-allocate a 1 MiB raw disk image for the virtio-blk persistence
# device (#335). The kernel's `block` module scans legacy PCI for the
# device-id 0x1042 slot and, if present, publishes a sector-oriented
# API to the Storage-2 layer. The image is created with a single
# `dd`-style seek-and-zero so every sector reads as 0x00 on a fresh
# container; #337's boot-time rehydrate treats that as "no checkpoint
# yet" and falls through to the baked metamodel.
RUN dd if=/dev/zero of=/disk.img bs=1M count=1 status=none

# Expose container port 8080 so `docker run -p 8080:8080` forwards
# host:8080 → container:8080 → QEMU's hostfwd → guest-kernel:80.
EXPOSE 8080

# Boot the image under QEMU:
#   -display none    headless
#   -serial stdio    banner to stdout
#   -no-reboot       exit on triple-fault instead of looping
#   -netdev user,id=net0,hostfwd=tcp::8080-:80
#                    SLiRP user-mode NAT — forwards container-local
#                    8080 into guest's :80 where smoltcp's HTTP
#                    listener lives (#264).
#   -device virtio-net-pci,netdev=net0,disable-legacy=on
#                    attach the guest NIC as modern-virtio (PCI device
#                    ID 0x1041) so the arest-kernel PCI scan (#262)
#                    finds it at boot. The default `disable-legacy=auto`
#                    on the `pc` machine yields a transitional device
#                    with the legacy ID 0x1000 — outside the kernel's
#                    0x1040..0x107F modern window — so the scan misses
#                    it and the HTTP listener falls back to loopback.
# timeout is deliberately omitted in this CMD — the kernel stays
# resident so `curl localhost:8080` from the host hits the HTTP
# server. For headless-CI runs use `boot-kernel.ps1 -Smoke` (banner
# check) or `-E2E` (banner + HTTP GET) which wrap this CMD in a
# timeout and assert the expected output.
CMD ["qemu-system-x86_64", \
     "-drive", "format=raw,file=/kernel.img", \
     "-serial", "stdio", \
     "-display", "none", \
     "-no-reboot", "-no-shutdown", \
     "-netdev", "user,id=net0,hostfwd=tcp::8080-:80", \
     "-device", "virtio-net-pci,netdev=net0,disable-legacy=on", \
     "-drive", "file=/disk.img,format=raw,if=none,id=disk0", \
     "-device", "virtio-blk-pci,drive=disk0,disable-legacy=on"]
