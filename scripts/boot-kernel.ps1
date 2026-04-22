# scripts/boot-kernel.ps1
#
# Build the arest-kernel disk image inside Docker (Linux), then boot
# it under QEMU (also inside Docker). Serial output streams to the
# terminal. Requires Docker Desktop on the host.
#
# Usage:
#   .\scripts\boot-kernel.ps1           # interactive boot (Ctrl-C to exit)
#   .\scripts\boot-kernel.ps1 -Smoke    # headless-CI run: boot, capture
#                                        # serial, assert banner, exit 0/1
#
# Interactive boot — what you'll see on success:
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
#
# Smoke mode (#208):
#   Boots the kernel headless under QEMU with a 20 s timeout, captures
#   every byte that comes out of COM1, and asserts every banner line
#   appears. Exits 0 on success, 1 with the captured log on failure.

param(
    [switch]$Smoke
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# docker writes BuildKit progress to stderr. PowerShell 5.1 wraps each
# stderr line in an ErrorRecord (NativeCommandError) when stderr is
# merged — which happens automatically when this script is piped or
# run under a harness that captures both streams. With
# $ErrorActionPreference = "Stop" those ErrorRecords throw before the
# build even starts. Relax error-action around native-exec calls and
# use exit code for control.
Write-Host "Building kernel image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel -f "$repoRoot\crates\arest-kernel-image\Dockerfile" $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

if ($Smoke) {
    Write-Host "`nBooting kernel in smoke mode (20 s cap)..." -ForegroundColor Cyan

    $containerName = "arest-kernel-smoke-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    # PS 5.1 Join-Path is two-arg only; chain to compose three segments.
    $targetDir = Join-Path $repoRoot "target"
    $logPath = Join-Path $targetDir "kernel-smoke.log"
    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

    # Run detached so we can terminate after the boot-banner window.
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        docker run --rm --name $containerName -d arest-kernel | Out-Null
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    if ($LASTEXITCODE -ne 0) { throw "docker run failed (exit $LASTEXITCODE)" }

    try {
        # Poll for banner completion, with a hard ceiling. The kernel
        # emits "idt:   int3 round-tripped" as the last boot-time line
        # before it parks in the REPL/network loop; once we see it
        # we have everything we need.
        $deadline = (Get-Date).AddSeconds(20)
        $log = ""
        while ((Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 500
            # docker logs writes container stderr to host stderr; merge
            # explicitly to a single stream for matching.
            $prevEAP = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $log = (docker logs $containerName 2>&1 | Out-String)
            } finally {
                $ErrorActionPreference = $prevEAP
            }
            if ($log -match "int3 round-tripped") { break }
        }
        $log | Out-File -FilePath $logPath -Encoding utf8

        $expected = @(
            "AREST kernel online",
            "target: x86_64-unknown-none",
            "heap:",
            "gdt:",
            "idt:",
            "EXCEPTION: BREAKPOINT",
            "int3 round-tripped"
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

        Write-Host "PASS: all banner phrases observed." -ForegroundColor Green
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

Write-Host "`nBooting under QEMU (Docker)..." -ForegroundColor Cyan
Write-Host "In another terminal: curl http://localhost:8080/" -ForegroundColor Yellow
Write-Host "Ctrl-C here to stop the kernel.`n" -ForegroundColor DarkGray
# -p 8080:8080 forwards host:8080 into the container, which QEMU
# then forwards into the guest's :80 via `-hostfwd=tcp::8080-:80`.
# Two forwards, one for each boundary — the whole path is:
#   host:8080 → container:8080 → guest_kernel:80 (smoltcp #264)
docker run --rm -p 8080:8080 arest-kernel
