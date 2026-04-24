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
# Smoke mode: proves the aarch64-UEFI scaffold boots end-to-end on
# QEMU-virt + AAVMF. Asserted banner phrases cover the three lines
# `entry_uefi_aarch64.rs::efi_main` writes before the `wfi` halt:
#   * "AREST kernel - aarch64-UEFI scaffold"
#   * "target: aarch64-unknown-uefi"
#   * "next:   ExitBootServices + memory map (follow-ups)"
#
# Remaining for #344 acceptance on aarch64 (tracked in follow-ups,
# matching the x86_64 arm's step-by-step progression):
#   * ExitBootServices + post-EBS PL011 cutover — aarch64 arm writes
#     PL011 MMIO directly from the start, so no _print swap is
#     needed; an explicit EBS call is still required to reclaim the
#     firmware memory map.
#   * GetMemoryMap consumption + aarch64 page-table abstraction.
#   * Kernel body modules (virtio, net, system) un-gated on aarch64
#     — blocked on the arch-neutral paging trait the x86_64 arm
#     also needs for virtio bring-up.

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
        -f "$repoRoot\crates\arest-kernel-image\Dockerfile.uefi-aarch64" `
        $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

if ($Smoke) {
    Write-Host "`nBooting aarch64-UEFI kernel in smoke mode (60 s cap)..." -ForegroundColor Cyan

    # aarch64 + AAVMF boot is noticeably slower than x86_64 + OVMF
    # under emulated QEMU — no KVM acceleration on x86 hosts, so
    # every ARM instruction goes through TCG. Budget 60 s for the
    # firmware scroll + banner, up from the x86 smoke's 30 s.
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
        # Poll for the final scaffold line ("next:   ExitBootServices"),
        # same pattern the x86_64 smoke uses: matching the last line
        # guarantees the log snapshot captures every prior banner
        # before we dump it to disk.
        $deadline = (Get-Date).AddSeconds(60)
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
        # show the exact line that dropped.
        $expected = @(
            "AREST kernel - aarch64-UEFI scaffold",
            "target: aarch64-unknown-uefi",
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
