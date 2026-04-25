# scripts/boot-kernel-uefi-aarch64.ps1
#
# aarch64 UEFI sibling of `boot-kernel-uefi.ps1`. Builds the arest
# kernel as `aarch64-unknown-uefi` (via `Dockerfile.uefi-aarch64`)
# and boots it under QEMU-aarch64 + AAVMF inside Docker. PL011
# serial output streams to the host terminal.
#
# Usage:
#   .\scripts\boot-kernel-uefi-aarch64.ps1            # interactive boot
#   .\scripts\boot-kernel-uefi-aarch64.ps1 -Smoke     # headless: assert banner
#
# Smoke mode (#366-#369):
#   Boots the aarch64-UEFI kernel under QEMU-virt + AAVMF, caps at
#   120 s (TCG emulation is slow + AAVMF boot surface + virtio bring-
#   up is heavier than a banner-only scaffold), captures every byte
#   of PL011 serial, and asserts every banner line the entry writes
#   pre- and post-ExitBootServices. Exits 0 on success, 1 with the
#   captured log on failure. Asserted banner lines cover:
#     * #366 memory bring-up: "mem: N frames usable (M MiB)" via
#       the UefiFrameAllocator singleton.
#     * #367 DMA pool carve: "dma: pool live (2 MiB UEFI memory-map
#       carve for virtio)".
#     * #368 MMIO walker: "virtio-mmio: walk OK (virtio-net: slot N
#       @ 0x..., virtio-blk: slot N @ 0x...)".
#     * #369 device bring-up: "virtio-net: driver online, MAC ..."
#       and "virtio-blk: driver online, N sectors ..., read-write".
#
# Remaining for full x86_64 parity:
#   * #337 mount / round-trip path — `block_storage` is
#     cfg(target_arch = "x86_64") gated; drops alongside an arch-
#     neutral block storage facade.
#   * GICv2/v3 + IDT-equivalent vector table for IRQ-driven smoltcp
#     parity (the x86_64 UEFI arm also doesn't reach this yet).

param(
    [switch]$Smoke
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# docker writes BuildKit progress to stderr; relax error-action
# around native-exec calls like the x86_64 smoke does.
Write-Host "Building aarch64-UEFI kernel image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-uefi-aarch64 `
        -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi-aarch64" `
        $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

if ($Smoke) {
    Write-Host "`nBooting aarch64-UEFI kernel in smoke mode (120 s cap)..." -ForegroundColor Cyan

    # aarch64 + AAVMF boot is noticeably slower than x86_64 + OVMF
    # under emulated QEMU — no KVM acceleration on x86 hosts, so
    # every ARM instruction goes through TCG. Budget 120 s for the
    # firmware scroll + banner + virtio bring-up, up from the
    # pre-#369 60 s (additional virtio-drivers init + MMIO queue
    # setup pushes the tail past 60 s on colder hosts).
    $containerName = "arest-kernel-uefi-aarch64-smoke-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    $targetDir = Join-Path $repoRoot "target"
    $logPath = Join-Path $targetDir "kernel-uefi-aarch64-smoke.log"
    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & docker run --rm --name $containerName -d arest-kernel-uefi-aarch64 | Out-Null
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    if ($LASTEXITCODE -ne 0) { throw "docker run failed (exit $LASTEXITCODE)" }

    try {
        # Poll for the final banner line ("next:   ExitBootServices"),
        # same pattern the x86_64 smoke uses: matching the last line
        # guarantees the log snapshot captures every prior banner
        # before we dump it to disk.
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

        # Banner phrases the aarch64 entry writes via PL011 MMIO.
        # Each line is asserted individually; partial-write regressions
        # show the exact line that dropped. The #369 lines are the
        # proof the MMIO transport + virtio bring-up reach the same
        # device-online state the x86_64-UEFI arm does.
        $expected = @(
            "AREST kernel - aarch64-UEFI scaffold",
            "target: aarch64-unknown-uefi",
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

        Write-Host "PASS: aarch64-UEFI scaffold banner observed." -ForegroundColor Green
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

Write-Host "`nBooting aarch64-UEFI kernel under QEMU + AAVMF (Docker)..." -ForegroundColor Cyan
Write-Host "Ctrl-C here to stop the kernel." -ForegroundColor DarkGray
Write-Host "Note: AAVMF prints its own boot banners before ours; AREST output appears after.`n" -ForegroundColor DarkGray
docker run --rm arest-kernel-uefi-aarch64
