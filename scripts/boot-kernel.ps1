# scripts/boot-kernel.ps1
#
# Build the arest-kernel disk image inside Docker (Linux), then boot
# it under QEMU (also inside Docker). Serial output streams to the
# terminal. Requires Docker Desktop on the host.
#
# Usage:
#   .\scripts\boot-kernel.ps1
#
# What you'll see on success:
#   AREST kernel online
#     target: x86_64-unknown-none
#     heap:   1 MiB static (#178)
#     gdt:    loaded with TSS + double-fault IST (#179)
#     idt:    breakpoint + double-fault + keyboard (#181)
#     pic:    remapped to 32+, keyboard (IRQ 1) unmasked
#     alloc: heap is live
#   EXCEPTION: BREAKPOINT
#     <stack frame>
#     idt:   int3 round-tripped through breakpoint handler
#
#   type on the keyboard — every keypress echoes over serial.

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

Write-Host "Building kernel image (Docker)..." -ForegroundColor Cyan
docker build -t arest-kernel -f "$repoRoot\crates\arest-kernel-image\Dockerfile" $repoRoot
if ($LASTEXITCODE -ne 0) { throw "Docker build failed" }

Write-Host "`nBooting under QEMU (Docker)..." -ForegroundColor Cyan
docker run --rm arest-kernel
