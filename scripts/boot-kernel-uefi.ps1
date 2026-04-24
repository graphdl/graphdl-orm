# scripts/boot-kernel-uefi.ps1
#
# UEFI sibling of `boot-kernel.ps1`. Builds the arest-kernel as a
# `x86_64-unknown-uefi` PE32+ image (via `Dockerfile.uefi`) and boots
# it under QEMU + OVMF inside Docker. Serial output streams to the
# host terminal.
#
# Usage:
#   .\scripts\boot-kernel-uefi.ps1            # interactive boot
#   .\scripts\boot-kernel-uefi.ps1 -Smoke     # headless: assert banner
#
# Smoke mode (#344 step 4c):
#   Boots the kernel under OVMF with a 30 s cap (boot-time UEFI
#   initialisation is slower than the BIOS path -- OVMF prints its
#   own banners before our entry runs), captures every byte of
#   serial, and asserts every banner line our entry writes pre- and
#   post-ExitBootServices, including the step-4c
#   "mem: N frames usable" line that proves the page-table + frame-
#   allocator singletons are live post-EBS. Exits 0 on success, 1
#   with the captured log on failure.
#
# Once step 4d lands the shared `kernel_run` handoff, the banner
# phrase list grows to match `boot-kernel.ps1`'s (heap / gdt / idt
# / blk / http) and the smoke gains parity with the BIOS one.

param(
    [switch]$Smoke
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# docker writes BuildKit progress to stderr; PowerShell 5.1 wraps each
# stderr line in an ErrorRecord (NativeCommandError) when streams are
# merged, which fires under `$ErrorActionPreference = "Stop"`. Mirror
# the BIOS script's pattern: relax error-action around native-exec
# calls and gate on $LASTEXITCODE.
Write-Host "Building UEFI kernel image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-uefi -f "$repoRoot\crates\arest-kernel-image\Dockerfile.uefi" $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

if ($Smoke) {
    Write-Host "`nBooting UEFI kernel in smoke mode (30 s cap)..." -ForegroundColor Cyan

    $containerName = "arest-kernel-uefi-smoke-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    $targetDir = Join-Path $repoRoot "target"
    $logPath = Join-Path $targetDir "kernel-uefi-smoke.log"
    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & docker run --rm --name $containerName -d arest-kernel-uefi | Out-Null
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    if ($LASTEXITCODE -ne 0) { throw "docker run failed (exit $LASTEXITCODE)" }

    try {
        # OVMF prints its own boot banners (TianoCore, UefiVersion, etc.)
        # before the firmware hands control to our PE32+. Poll for the
        # step-4c "mem: N frames usable" marker, which is the latest
        # line our entry prints before parking -- it's written only
        # after the page-table + frame-allocator singletons are live,
        # so it proves every step from ConOut through EBS through the
        # post-EBS 16550 cutover through init_memory worked.
        $deadline = (Get-Date).AddSeconds(30)
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
            # Match against the most recent line the scaffold writes.
            # Once step 4d boots through `kernel_run`, swap the marker
            # for "int3 round-tripped" -- the BIOS smoke's same beacon.
            if ($log -match "frames usable") { break }
        }
        $log | Out-File -FilePath $logPath -Encoding utf8

        # Step 4 banner phrases. Each line the entry writes is asserted
        # individually so a partial-write regression is easy to spot.
        # The "post-EBS" line proves the 16550 COM1 fall-back works
        # after firmware ConOut is invalid; the "mem:" line proves
        # step 4c's init_memory + page-table / frame-allocator
        # singletons are live post-EBS.
        $expected = @(
            "AREST kernel - UEFI scaffold (#344)",
            "step 4 of 8: ExitBootServices + post-EBS serial",
            "pre-EBS:  ConOut active",
            "SSE enabled",
            "post-EBS: 16550 COM1 active",
            "frames usable",
            "post-EBS heap live (sum 0..16 = 120)",
            "system::init() completed (arest engine live on UEFI)",
            "Engine::default() constructed (runtime live on UEFI)"
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

        Write-Host "PASS: UEFI scaffold banner observed." -ForegroundColor Green
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

Write-Host "`nBooting UEFI kernel under QEMU + OVMF (Docker)..." -ForegroundColor Cyan
Write-Host "Ctrl-C here to stop the kernel." -ForegroundColor DarkGray
Write-Host "Note: OVMF prints its own boot banners before ours; AREST output appears after.`n" -ForegroundColor DarkGray
docker run --rm arest-kernel-uefi
