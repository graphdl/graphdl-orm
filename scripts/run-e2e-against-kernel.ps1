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

Write-Host "[#656 e2e] Building UEFI kernel image (server profile)..." -ForegroundColor Cyan
# Use the lean server profile (no Slint runtime, no UI bundle, no
# REPL/PS-2 keyboard) — that's the profile #657's smoke verifies
# host-curl-reachable, and dropping the launcher hand-off lets
# `loop { net::poll(); pause }` drive smoltcp without competing
# with the Slint event loop's idle pump.
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-server -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi-server" $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

Write-Host "[#656 e2e] Booting kernel container '$containerName' on 127.0.0.1:8080..." -ForegroundColor Cyan
# Bind explicitly to 127.0.0.1 (not 0.0.0.0) so PowerShell host-side
# probes hit IPv4 loopback directly rather than going through the
# WSL2 VM's bridge. See #657 — same Windows/Docker-Desktop quirk
# that caused "Empty reply from server" on the smoke harness path.
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    & docker run --rm --name $containerName -d -p 127.0.0.1:8080:8080 arest-kernel-server | Out-Null
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
    # Wait for 127.0.0.1:8080 to accept connections. Any HTTP response
    # (incl. 404 at `/`) means the kernel is listening — that's the
    # readiness condition. Connection refused or timeout means the
    # net stack hasn't settled yet.
    #
    # Use curl.exe rather than Invoke-WebRequest. IWR depends on IE/Edge
    # COM interfaces that PowerShell 5.1 in `-NonInteractive` mode can't
    # initialise — silent failure looks like an Empty-reply timeout.
    # See #657 commit b61652dd for the same fix on the smoke harness.
    Write-Host "[#656 e2e] Waiting up to ${BootTimeoutSec}s for 127.0.0.1:8080 to accept connections..." -ForegroundColor Cyan
    $deadline = (Get-Date).AddSeconds($BootTimeoutSec)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        $code = & curl.exe -s -m 2 -o NUL -w "%{http_code}" "http://127.0.0.1:8080/" 2>$null
        if ($LASTEXITCODE -eq 0 -and $code -match '^[0-9]+$' -and [int]$code -gt 0) {
            $ready = $true
            break
        }
        Start-Sleep -Milliseconds 500
    }

    if (-not $ready) {
        Write-Warning ("[#656 e2e] 127.0.0.1:8080 did not open within ${BootTimeoutSec}s. " +
            "Kernel may not have reached `server: net+http loop running` yet; " +
            "raise -BootTimeoutSec or check `docker logs $containerName`.")
        Stop-KernelContainer
        exit 1
    }

    Write-Host "[#656 e2e] 127.0.0.1:8080 is up. Running vitest suite..." -ForegroundColor Green
    $env:BASE = "http://127.0.0.1:8080"
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
