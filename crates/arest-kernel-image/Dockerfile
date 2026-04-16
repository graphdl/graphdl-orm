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

# Install nightly + components the kernel and image-builder need
RUN rustup toolchain install nightly --profile minimal \
    && rustup component add rust-src --toolchain nightly \
    && rustup target add x86_64-unknown-none --toolchain nightly

WORKDIR /build

# Copy kernel crate
COPY crates/arest-kernel/Cargo.toml crates/arest-kernel/Cargo.toml
COPY crates/arest-kernel/src/ crates/arest-kernel/src/

# Copy image-builder crate
COPY crates/arest-kernel-image/Cargo.toml crates/arest-kernel-image/Cargo.toml
COPY crates/arest-kernel-image/src/ crates/arest-kernel-image/src/

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

# Boot the image. -display none = headless, -serial stdio = banner to
# stdout, -no-reboot = exit on triple-fault instead of looping.
# timeout 10s catches infinite hlt loops so `docker run` always exits.
CMD ["timeout", "10", "qemu-system-x86_64", \
     "-drive", "format=raw,file=/kernel.img", \
     "-serial", "stdio", \
     "-display", "none", \
     "-no-reboot", "-no-shutdown"]
