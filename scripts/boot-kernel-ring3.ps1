# scripts/boot-kernel-ring3.ps1
#
# Sec-6 ring-3 smoke harness (#333). Builds the kernel with
# `--features ring3-smoke` (via `Dockerfile.ring3`), boots under
# QEMU with the `isa-debug-exit` device on port 0xF4, and asserts:
#
#   1. The serial output contains the ring-3 descent banner +
#      observability prints from each syscall the payload makes:
#        - "userspace: mapped user text + stack, descending to ring 3"
#        - "syscall: SYS_yield"
#        - "syscall: SYS_system(key=..., input=0 B) -> ..."
#        - "syscall: SYS_exit(0x10)"
#   2. QEMU's process exit code is 33, i.e. (0x10 << 1) | 1, which
#      is what the device writes when ring 3 calls SYS_exit(SUCCESS).
#
# Either failure mode is reported with the captured serial log so
# the regression is easy to read.
#
# Usage:
#   .\scripts\boot-kernel-ring3.ps1
#   (defaults to smoke mode — no interactive variant for ring 3
#   since the payload halts after a few syscalls.)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

Write-Host "Building ring-3 smoke kernel image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-ring3 -f "$repoRoot\crates\arest-kernel-image\Dockerfile.ring3" $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

Write-Host "`nBooting ring-3 smoke under QEMU (10 s cap)..." -ForegroundColor Cyan

$containerName = "arest-kernel-ring3-$([guid]::NewGuid().ToString('N').Substring(0,8))"
$targetDir = Join-Path $repoRoot "target"
$logPath = Join-Path $targetDir "kernel-ring3-smoke.log"
New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

# Run in the foreground so we can read QEMU's exit code directly.
# The container's CMD invokes `qemu-system-x86_64 ...
# -device isa-debug-exit,...`; QEMU exits when the kernel writes
# to port 0xF4. The container then exits with QEMU's exit code.
# Docker propagates the exit code to `docker run`.
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
$log = ""
try {
    # 10 s timeout — payload halts in milliseconds normally; the
    # cap is just a safety net against a stuck boot.
    $log = (docker run --rm --name $containerName arest-kernel-ring3 2>&1 | Out-String)
} finally {
    $ErrorActionPreference = $prevEAP
}
$qemuExit = $LASTEXITCODE
$log | Out-File -FilePath $logPath -Encoding utf8

Write-Host "Serial log: $logPath"
Write-Host "QEMU exit code: $qemuExit"

# Assertion 1: serial banner phrases.
# Each phrase corresponds to a different point in the ring-3 descent
# + syscall path. If any are missing, that part of the pipeline broke.
#
# The "0xffff800000000000" line proves UserBuf::from_raw rejected a
# kernel-half pointer the ring-3 payload deliberately handed in (via
# SYS_fetch). The dispatcher's early-trace println is the only place
# this rejection becomes visible — the per-arm prints don't run when
# UserBuf::from_raw bails early.
$expected = @(
    "userspace: mapped user text + stack, descending to ring 3",
    "syscall: SYS_yield",
    "syscall: SYS_system",
    "syscall: enter nr=1 a0=0xffff800000000000",
    "syscall: SYS_exit(0x10)"
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
    Write-Host "`n--- captured serial log ---"
    Write-Host $log
    exit 1
}

# Assertion 2: QEMU exit code 33 = (0x10 << 1) | 1.
# This is the isa-debug-exit device's encoding: the kernel writes
# `SUCCESS=0x10` to port 0xF4 from inside `userspace::halt_on_exit`,
# QEMU multiplies-and-bumps to 33, the container exits with 33,
# Docker propagates 33 here.
$expectedExit = 33
if ($qemuExit -ne $expectedExit) {
    Write-Host "FAIL: QEMU exited $qemuExit (expected $expectedExit = (0x10 << 1) | 1)." -ForegroundColor Red
    Write-Host "`n--- captured serial log ---"
    Write-Host $log
    exit 1
}

Write-Host "PASS: ring-3 descent + SYS_yield + SYS_system + SYS_exit(SUCCESS)." -ForegroundColor Green
exit 0
