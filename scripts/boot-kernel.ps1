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
Write-Host "In another terminal: curl http://localhost:8080/" -ForegroundColor Yellow
Write-Host "Ctrl-C here to stop the kernel.`n" -ForegroundColor DarkGray
# -p 8080:8080 forwards host:8080 into the container, which QEMU
# then forwards into the guest's :80 via `-hostfwd=tcp::8080-:80`.
# Two forwards, one for each boundary — the whole path is:
#   host:8080 → container:8080 → guest_kernel:80 (smoltcp #264)
docker run --rm -p 8080:8080 arest-kernel
