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
# Smoke mode (#344 step 4 waves 1-6 + #270/#271):
#   Boots the kernel under OVMF with a 30 s cap (boot-time UEFI
#   initialisation is slower than the BIOS path -- OVMF prints its
#   own banners before our entry runs), captures every byte of
#   serial, and asserts every banner line our entry writes pre- and
#   post-ExitBootServices. Exits 0 on success, 1 with the captured
#   log on failure. Asserted banner lines cover:
#     * step 4b: ExitBootServices + post-EBS 16550 serial cutover
#     * step 4d prep: SSE enable pre-EBS (5dc246a)
#     * step 4c: init_memory from UEFI memory map -> frames usable
#     * step 4d wave 3: post-EBS static-BSS heap (5b74f2a)
#     * step 4d wave 4: AREST engine init on UEFI (8ea0528)
#     * step 4d wave 5: wasmi executes user WASM (58cf113)
#     * #270/#271 shim: 10 Doom host imports bound (f8c11d2)
#     * step 4d wave 6: framebuffer::install + triple-buffer paint
#       smoke on GOP (#269 BIOS path now reachable on UEFI)
#
# Remaining for step 4d completion: kernel_run() handoff — requires
# virtio / block / net to compile on UEFI (currently gated) or a
# UEFI-specific kernel_run_uefi that skips those subsystems.

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
        # FINAL line the entry prints before halt -- "next: kernel_run
        # handoff (step 4d)" -- rather than an early line like "frames
        # usable". Without that, the docker-logs snapshot can race the
        # kernel's later output: the matcher fires on the early line,
        # the loop breaks, and the snapshot we then write to disk is
        # missing everything the kernel printed after. Polling on the
        # last line guarantees the snapshot includes every banner.
        #
        # Once step 4d boots through `kernel_run`, swap the marker for
        # "int3 round-tripped" -- the BIOS smoke's same beacon.
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
            if ($log -match "next:\s+kernel_run handoff") { break }
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
            "tiny module executed, main() = 42",
            "10 host imports bound to wasmi::Linker",
            "gop:      ",
            "gop-mmio: wrote 320x200, readback sum=0xffff8300",
            "fb:       paint smoke OK, presents=2",
            "doom-blit: synthetic 640x400 BGRA frame blitted"
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
