# scripts/run-e2e-against-worker.ps1
#
# Worker-side analogue of `run-e2e-against-kernel.ps1` (#656 / #624).
# Boots `wrangler dev` locally, waits for 127.0.0.1:8787 to accept
# connections, runs the BASE-parameterized AI-dispatch e2e suite
# (`src/tests/e2e-ai.test.ts`) against it, then halts wrangler.
#
# This is the unblock-#642 invocation: it exercises the migrated
# /arest/extract (#639) and /arest/chat (#640) handlers end-to-end
# against the same Cloudflare Worker bundle that ships to api.auto.dev,
# without requiring real AI_GATEWAY credentials. The 503 path the
# handlers take when AI_GATEWAY_URL/TOKEN are absent is the documented
# cross-target contract per #620 — that's what this script verifies.
#
# Usage:
#   .\scripts\run-e2e-against-worker.ps1
#
# To run against the deployed worker instead (no wrangler dev),
# bypass this script entirely:
#   $env:BASE = "https://api.auto.dev"
#   yarn test src/tests/e2e-ai.test.ts
#
# Env overrides:
#   E2E_BOOT_TIMEOUT_SEC   how long to wait for 127.0.0.1:8787 to open
#                          (default 30; wrangler dev usually binds in
#                          ~3-8 s on a warm cache)
#   E2E_KEEP_WRANGLER      '1' to leave wrangler dev running after the
#                          suite finishes (handy for manual probing)

param(
    [int]$BootTimeoutSec = 0,
    [switch]$KeepWrangler
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

if ($BootTimeoutSec -le 0) {
    if ($env:E2E_BOOT_TIMEOUT_SEC) {
        $BootTimeoutSec = [int]$env:E2E_BOOT_TIMEOUT_SEC
    } else {
        $BootTimeoutSec = 30
    }
}
if (-not $KeepWrangler.IsPresent -and $env:E2E_KEEP_WRANGLER -eq '1') {
    $KeepWrangler = [switch]::Present
}

# Bind wrangler to 127.0.0.1:8787 explicitly. Same Windows/Docker-Desktop
# IPv6/IPv4 quirk that bit #657 — the host curl probe wants v4 loopback
# direct, not the v6 default wrangler picks if you let it.
$wranglerHost = "127.0.0.1"
$wranglerPort = 8787

Write-Host "[#642 e2e] Booting wrangler dev on ${wranglerHost}:${wranglerPort}..." -ForegroundColor Cyan

$logFile = Join-Path $env:TEMP "arest-wrangler-dev-$([guid]::NewGuid().ToString('N').Substring(0,8)).log"
Write-Host "[#642 e2e] wrangler log: $logFile" -ForegroundColor DarkGray

# Spawn wrangler dev as a background process. We capture stdout+stderr
# to a temp file so a startup failure (port in use, wasm build error)
# is debuggable without staring at a stale terminal. wrangler keeps
# running until Stop-Process — no idle-timeout flag in v4.
Push-Location $repoRoot
try {
    $wranglerProc = Start-Process `
        -FilePath "yarn" `
        -ArgumentList "dev","--ip",$wranglerHost,"--port",$wranglerPort `
        -RedirectStandardOutput $logFile `
        -RedirectStandardError "$logFile.err" `
        -PassThru `
        -NoNewWindow `
        -WorkingDirectory $repoRoot
} finally {
    Pop-Location
}

if (-not $wranglerProc) {
    throw "Failed to spawn wrangler dev"
}

# Best-effort cleanup: always try to halt wrangler on exit unless the
# operator asked us to keep it. We use try/finally rather than a trap
# so an explicit Ctrl-C still triggers the halt.
$wranglerHalted = $false
function Stop-Wrangler {
    if ($script:wranglerHalted) { return }
    if ($KeepWrangler.IsPresent) {
        Write-Host "[#642 e2e] -KeepWrangler set; leaving wrangler PID $($wranglerProc.Id) running." -ForegroundColor Yellow
        $script:wranglerHalted = $true
        return
    }
    Write-Host "[#642 e2e] Halting wrangler dev (PID $($wranglerProc.Id))..." -ForegroundColor Cyan
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        # `wrangler dev` spawns a child workerd process — kill the tree.
        Stop-Process -Id $wranglerProc.Id -Force -ErrorAction SilentlyContinue
        # Best-effort: drop any stray workerd processes that escaped
        # the parent kill (esbuild/miniflare can briefly orphan).
        Get-Process -Name workerd -ErrorAction SilentlyContinue | ForEach-Object {
            Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
        }
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    $script:wranglerHalted = $true
}

try {
    # Wait for the wrangler dev port to accept connections. Any HTTP
    # response (incl. 404 at `/`) means wrangler is listening.
    #
    # Use curl.exe rather than Invoke-WebRequest. IWR depends on IE/Edge
    # COM interfaces that PowerShell 5.1 in `-NonInteractive` mode can't
    # initialise — silent failure looks like an Empty-reply timeout.
    # See #657 commit b61652dd for the same fix on the kernel boot harness.
    Write-Host "[#642 e2e] Waiting up to ${BootTimeoutSec}s for ${wranglerHost}:${wranglerPort} to accept connections..." -ForegroundColor Cyan
    $deadline = (Get-Date).AddSeconds($BootTimeoutSec)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        # Detect early death — if wrangler exited (port-in-use, build
        # error, etc.), abort fast rather than waiting the full timeout.
        if ($wranglerProc.HasExited) {
            Write-Warning "[#642 e2e] wrangler dev exited before ready (exit code $($wranglerProc.ExitCode)). Tail of log:"
            if (Test-Path $logFile) { Get-Content -Tail 30 $logFile | Write-Host }
            if (Test-Path "$logFile.err") { Get-Content -Tail 30 "$logFile.err" | Write-Host }
            exit 1
        }
        # `-o NUL` discards the body via the Windows null device — same
        # form the kernel-side harness uses (#657 b61652dd). curl.exe's
        # `/dev/null` shim works too but `NUL` is the native PS5 idiom
        # and avoids any cygwin-path-translation surprises.
        $code = & curl.exe -s -m 2 -o NUL -w "%{http_code}" "http://${wranglerHost}:${wranglerPort}/" 2>$null
        if ($LASTEXITCODE -eq 0 -and $code -match '^[0-9]+$' -and [int]$code -gt 0) {
            $ready = $true
            break
        }
        Start-Sleep -Milliseconds 500
    }

    if (-not $ready) {
        Write-Warning ("[#642 e2e] ${wranglerHost}:${wranglerPort} did not open within ${BootTimeoutSec}s. " +
            "wrangler dev may still be building wasm / fetching deps. " +
            "Raise -BootTimeoutSec or check $logFile.")
        if (Test-Path $logFile) { Get-Content -Tail 30 $logFile | Write-Host }
        Stop-Wrangler
        exit 1
    }

    Write-Host "[#642 e2e] ${wranglerHost}:${wranglerPort} is up. Running vitest suite..." -ForegroundColor Green
    $env:BASE = "http://${wranglerHost}:${wranglerPort}"
    Push-Location $repoRoot
    try {
        # `yarn test` resolves to `vitest run` per package.json.
        $prevEAP = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            yarn test src/tests/e2e-ai.test.ts
        } finally {
            $ErrorActionPreference = $prevEAP
        }
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
        Remove-Item Env:\BASE -ErrorAction SilentlyContinue
    }

    Stop-Wrangler
    if ($exitCode -ne 0) {
        Write-Error "[#642 e2e] vitest exited $exitCode against the worker."
        exit $exitCode
    }
    Write-Host "[#642 e2e] AI-dispatch e2e against worker: PASS." -ForegroundColor Green
} finally {
    Stop-Wrangler
}
