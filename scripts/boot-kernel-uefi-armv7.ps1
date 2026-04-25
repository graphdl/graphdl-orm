# scripts/boot-kernel-uefi-armv7.ps1
#
# armv7 UEFI sibling of `boot-kernel-uefi-aarch64.ps1` (and the
# x86_64-UEFI `boot-kernel-uefi.ps1`). Builds the arest kernel as
# `armv7-unknown-uefi` (via `Dockerfile.uefi-armv7`) and boots it
# under qemu-system-arm + ArmVirtPkg inside Docker. PL011 serial
# output streams to the host terminal.
#
# Usage:
#   .\scripts\boot-kernel-uefi-armv7.ps1            # interactive boot
#   .\scripts\boot-kernel-uefi-armv7.ps1 -Smoke     # headless: assert banner
#
# Smoke mode (#389):
#   Boots the armv7-UEFI kernel under qemu-system-arm + ArmVirtPkg,
#   caps at 120 s (TCG emulation is slow + ArmVirtPkg boot surface +
#   virtio bring-up is heavier than a banner-only scaffold),
#   captures every byte of PL011 serial, and asserts every banner
#   line the entry writes pre- and post-ExitBootServices. Exits 0
#   on success, 1 with the captured log on failure. Asserted banner
#   lines mirror the aarch64 smoke shape (parity is the whole point
#   of #389):
#     * Memory bring-up: "mem: N frames usable (M MiB)" via the
#       UefiFrameAllocator singleton (#387 widened to armv7 in
#       `arch::armv7::memory::init`).
#     * DMA pool carve: "dma: pool live (2 MiB UEFI memory-map
#       carve for virtio)" (#387 + #388 widening).
#     * MMIO walker: "virtio-mmio: walk OK (virtio-net: slot N
#       @ 0x..., virtio-blk: slot N @ 0x...)" (#388 widened the
#       transport to armv7).
#     * Device bring-up: "virtio-net: driver online, MAC ..." and
#       "virtio-blk: driver online, N sectors ..., read-write"
#       (#389, the armv7 entry harness landing).
#
# Remaining for full x86_64 parity (same gap the aarch64 arm has):
#   * #337 mount / round-trip path — `block_storage` is
#     cfg(target_arch = "x86_64") gated; drops alongside an arch-
#     neutral block storage facade.
#   * GIC + IDT-equivalent vector table for IRQ-driven smoltcp
#     parity (the aarch64 arm also doesn't reach this yet).
#
# KNOWN BLOCKER (#389 final report):
#   The Debian `qemu-efi-arm` package ships an AAVMF32_CODE.fd that
#   does NOT include the VirtIO Block / FAT / Partition / Loaded-
#   Image-Protocol driver chain. Verified via `strings -a` returning
#   zero `virtio` matches in the firmware blob. At runtime BdsDxe
#   finds the virtio-blk-device but logs `Boot0002 "UEFI Non-Block
#   Boot Device" from VenHw(...): Unsupported` and falls back to PXE
#   (also unconfigured), then halts. The kernel itself never starts.
#   See `Dockerfile.uefi-armv7`'s top-of-file comment for the full
#   blocker writeup + resolution paths.
#
#   THE SMOKE WILL FAIL UNTIL the firmware blocker is resolved (a
#   fuller ArmVirtPkg32 build, U-Boot's UEFI sub-system, or another
#   firmware that includes VirtioBlkDxe). The kernel-side artefact
#   is correct — verified clean against the four-target build matrix
#   (armv7-unknown-uefi, aarch64-unknown-uefi, x86_64-unknown-uefi,
#   x86_64-unknown-none) per the brief's verification step 1 + 2.

param(
    [switch]$Smoke
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# docker writes BuildKit progress to stderr; relax error-action
# around native-exec calls like the aarch64 / x86_64 smokes do.
Write-Host "Building armv7-UEFI kernel image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-uefi-armv7 `
        -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi-armv7" `
        $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

if ($Smoke) {
    Write-Host "`nBooting armv7-UEFI kernel in smoke mode (120 s cap)..." -ForegroundColor Cyan

    # qemu-system-arm + ArmVirtPkg under TCG is comparable in speed
    # to qemu-system-aarch64 + AAVMF — no KVM acceleration on x86
    # hosts, every ARM instruction goes through TCG. Budget 120 s
    # for the firmware scroll + banner + virtio bring-up, matching
    # the aarch64 smoke. ArmVirtPkg's 32-bit boot path is a hair
    # leaner than AAVMF's 64-bit path so the timing should stay
    # comfortably under the cap.
    $containerName = "arest-kernel-uefi-armv7-smoke-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    $targetDir = Join-Path $repoRoot "target"
    $logPath = Join-Path $targetDir "kernel-uefi-armv7-smoke.log"
    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & docker run --rm --name $containerName -d arest-kernel-uefi-armv7 | Out-Null
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    if ($LASTEXITCODE -ne 0) { throw "docker run failed (exit $LASTEXITCODE)" }

    try {
        # Poll for the final banner line ("next:   ExitBootServices"),
        # same pattern the aarch64 / x86_64 smokes use: matching the
        # last line guarantees the log snapshot captures every prior
        # banner before we dump it to disk.
        $deadline = (Get-Date).AddSeconds(120)
        $log = ""
        while ((Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 500
            $prevEAP = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $log = (docker logs $containerName 2>&1 | Out-String)
            } finally {
                $ErrorActionPreference = $prevEAP
            }
            if ($log -match "ExitBootServices \+ memory map") { break }
        }
        $log | Out-File -FilePath $logPath -Encoding utf8

        # Banner phrases the armv7 entry writes via PL011 MMIO.
        # Each line is asserted individually; partial-write regressions
        # show the exact line that dropped. Mirrors the aarch64 smoke
        # phrase set exactly except for the target line — same shape,
        # different ISA / target name.
        $expected = @(
            "AREST kernel - armv7-UEFI scaffold",
            "target: armv7-unknown-uefi",
            "pre-EBS:  PL011 MMIO active at 0x0900_0000",
            "post-EBS: PL011 MMIO survives",
            "frames usable",
            "dma:      pool live (2 MiB UEFI memory-map carve for virtio)",
            "virtio-mmio: walk OK (virtio-net:",
            "virtio-net: driver online, MAC",
            "virtio-blk: driver online,",
            "next:   ExitBootServices + memory map"
        )
        $missing = @()
        foreach ($phrase in $expected) {
            if ($log -notmatch [regex]::Escape($phrase)) {
                $missing += $phrase
            }
        }

        if ($missing.Count -gt 0) {
            Write-Host "FAIL: missing banner phrases:" -ForegroundColor Red
            $missing | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
            Write-Host "`n--- captured serial log ($logPath) ---"
            Write-Host $log
            exit 1
        }

        Write-Host "PASS: armv7-UEFI scaffold banner observed." -ForegroundColor Green
        Write-Host "Serial log: $logPath"
        exit 0
    }
    finally {
        $prevEAP = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            docker stop $containerName 2>&1 | Out-Null
        } finally {
            $ErrorActionPreference = $prevEAP
        }
    }
}

Write-Host "`nBooting armv7-UEFI kernel under qemu-system-arm + ArmVirtPkg (Docker)..." -ForegroundColor Cyan
Write-Host "Ctrl-C here to stop the kernel." -ForegroundColor DarkGray
Write-Host "Note: ArmVirtPkg prints its own boot banners before ours; AREST output appears after.`n" -ForegroundColor DarkGray
docker run --rm arest-kernel-uefi-armv7
