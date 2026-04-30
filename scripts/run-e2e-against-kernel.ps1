# scripts/run-e2e-against-kernel.ps1
#
# Boot the arest-kernel UEFI server profile, wait for :8080 to open on
# the host, run the BASE-parameterized HATEOAS e2e suite against it,
# halt the kernel.
#
# This is the unblock-#624 invocation: it threads `BASE=http://localhost:8080`
# into the same vitest suite that runs against `https://api.auto.dev`
# (the worker) so the contract-parity claim from
# `_reports/kernel-hateoas-gap.md` is verified end-to-end.
#
# Assumes #655's net fix has landed (DHCPv4 settles deterministically
# under the QEMU container). Soft-warns and exits 1 if :8080 doesn't
# open within 60 s — this script does NOT promote that warning to a
# hard fail because intermittent network settling is the known
# pre-#655 failure mode and we don't want CI flakes.
#
# Usage:
#   .\scripts\run-e2e-against-kernel.ps1
#
# Env overrides:
#   E2E_BOOT_TIMEOUT_SEC   how long to wait for :8080 (default 60)
#   E2E_KEEP_CONTAINER     '1' to leave the kernel container running
#                          after the suite (handy for debugging)

param(
    [int]$BootTimeoutSec = 0,
    [switch]$KeepContainer
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

if ($BootTimeoutSec -le 0) {
    if ($env:E2E_BOOT_TIMEOUT_SEC) {
        $BootTimeoutSec = [int]$env:E2E_BOOT_TIMEOUT_SEC
    } else {
        $BootTimeoutSec = 60
    }
}
if (-not $KeepContainer.IsPresent -and $env:E2E_KEEP_CONTAINER -eq '1') {
    $KeepContainer = [switch]::Present
}

$containerName = "arest-kernel-e2e-$([guid]::NewGuid().ToString('N').Substring(0,8))"

Write-Host "[#656 e2e] Building UEFI kernel image..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-uefi -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi" $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

Write-Host "[#656 e2e] Booting kernel container '$containerName' on :8080..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    & docker run --rm --name $containerName -d -p 8080:8080 arest-kernel-uefi | Out-Null
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "docker run failed (exit $LASTEXITCODE)" }

# Best-effort cleanup: always try to halt the container on exit unless
# the operator asked us to keep it. We use a try/finally rather than
# a trap so an explicit Ctrl-C still triggers the halt.
$kernelHalted = $false
function Stop-KernelContainer {
    if ($script:kernelHalted) { return }
    if ($KeepContainer.IsPresent) {
        Write-Host "[#656 e2e] -KeepContainer set; leaving '$containerName' running." -ForegroundColor Yellow
        $script:kernelHalted = $true
        return
    }
    Write-Host "[#656 e2e] Halting kernel container '$containerName'..." -ForegroundColor Cyan
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & docker stop $containerName 2>&1 | Out-Null
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    $script:kernelHalted = $true
}

try {
    # Wait for :8080 to accept connections. Any HTTP response (incl.
    # 404 at `/`) means the kernel is listening — that's the
    # readiness condition. Connection refused or timeout means the
    # net stack hasn't settled yet (#655).
    Write-Host "[#656 e2e] Waiting up to ${BootTimeoutSec}s for :8080 to accept connections..." -ForegroundColor Cyan
    $deadline = (Get-Date).AddSeconds($BootTimeoutSec)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        $prevEAP = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            $resp = Invoke-WebRequest -Uri "http://localhost:8080/" -TimeoutSec 2 -UseBasicParsing -ErrorAction SilentlyContinue
            if ($null -ne $resp) { $ready = $true }
        } catch {
            # Connection refused / timeout — keep polling.
        } finally {
            $ErrorActionPreference = $prevEAP
        }
        if ($ready) { break }
        Start-Sleep -Milliseconds 500
    }

    if (-not $ready) {
        Write-Warning ("[#656 e2e] :8080 did not open within ${BootTimeoutSec}s. " +
            "This is the #655 net-settling regime; the e2e suite will skip cleanly. " +
            "Re-run with -BootTimeoutSec 120 or land #655's net fix and retry.")
        # Soft-warn, not throw: the test suite itself handles
        # connection-refused as a clean skip, so we still exit 0 to
        # let the operator see the skip rather than a script failure.
        Stop-KernelContainer
        exit 1
    }

    Write-Host "[#656 e2e] :8080 is up. Running vitest suite..." -ForegroundColor Green
    $env:BASE = "http://localhost:8080"
    Push-Location $repoRoot
    try {
        # `yarn test` resolves to `vitest run` per package.json.
        $prevEAP = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            yarn test src/tests/e2e-hateoas.test.ts
        } finally {
            $ErrorActionPreference = $prevEAP
        }
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
        Remove-Item Env:\BASE -ErrorAction SilentlyContinue
    }

    Stop-KernelContainer
    if ($exitCode -ne 0) {
        Write-Error "[#656 e2e] vitest exited $exitCode against the kernel."
        exit $exitCode
    }
    Write-Host "[#656 e2e] HATEOAS e2e against kernel: PASS." -ForegroundColor Green
} finally {
    Stop-KernelContainer
}
