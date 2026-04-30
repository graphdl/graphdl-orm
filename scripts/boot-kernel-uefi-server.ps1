# scripts/boot-kernel-uefi-server.ps1
#
# Server-profile sibling of `boot-kernel-uefi.ps1`. Builds the kernel
# via `crates/arest-kernel/Dockerfile.uefi-server` (which selects
# `--no-default-features --features server` per #655) and boots it
# under QEMU + OVMF inside Docker, then asserts the kernel is reachable
# from the host on http://localhost:8080/.
#
# Profile contract (#600 + #627 + #655):
#   * Slint runtime + slint-build are NOT pulled into the build graph.
#   * REPL + PS/2 keyboard subsystem are NOT pulled into the build graph.
#   * `entry_uefi.rs::kernel_run_uefi` runs `loop { net::poll(); pause }`
#     instead of handing off to the Slint launcher event loop.
#   * `net::now()` reads the PIT-backed `arch::time::now_ms()` so
#     smoltcp's DHCPv4 retry timers align with wall-clock real-network
#     latency (not the per-poll counter that previously raced ahead by
#     ~1000x and starved SLiRP's DHCP server response window).
#
# Usage:
#   .\scripts\boot-kernel-uefi-server.ps1            # interactive boot
#   .\scripts\boot-kernel-uefi-server.ps1 -Smoke     # headless: assert
#                                                      banner + host curl
#
# Smoke mode succeeds when:
#   1. The kernel banner reaches "server: net+http loop running"
#      (the last line entry_uefi writes before entering the net+http
#      drainer; the slint launcher banner is unreachable on this
#      profile).
#   2. `Invoke-WebRequest http://localhost:8080/` returns within
#      90 s — the host-reachable HATEOAS read surface needed for
#      #624 / the apis e2e suite.
#
# Failure mode: timed out before either banner OR host curl. Exits 1
# with the captured serial log.

param(
    [switch]$Smoke
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# docker writes BuildKit progress to stderr; PowerShell 5.1 wraps each
# stderr line in an ErrorRecord (NativeCommandError) when streams are
# merged, which fires under `$ErrorActionPreference = "Stop"`. Mirror
# the BIOS / non-server scripts: relax error-action around native-exec
# calls and gate on $LASTEXITCODE.
Write-Host "Building UEFI kernel server image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-server -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi-server" $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

if ($Smoke) {
    # 90 s cap. The kernel banner reaches "server: net+http loop
    # running" within ~25 s on a cold container (no Doom WASM
    # instantiate, no Slint launcher splash); the curl-from-host
    # phase then has the remaining budget to settle DHCPv4 on
    # smoltcp's PIT-backed clock and answer the first GET / .
    Write-Host "`nBooting UEFI kernel server in smoke mode (90 s cap)..." -ForegroundColor Cyan

    $containerName = "arest-kernel-server-smoke-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    $targetDir = Join-Path $repoRoot "target"
    $logPath = Join-Path $targetDir "kernel-uefi-server-smoke.log"
    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        # -p 127.0.0.1:8080:8080: bind the host side of the port mapping
        # to IPv4 loopback explicitly. The default `-p 8080:8080` form
        # binds to `0.0.0.0`, which Docker Desktop on Windows reaches
        # only via the WSL2 VM's IPv4 address — `Invoke-WebRequest
        # http://127.0.0.1:8080/` then probes a port that isn't
        # forwarded back, and times out with "Empty reply from server".
        # The explicit `127.0.0.1:` host binding (or use of `-4`-flagged
        # curl against the same address) routes the host-side connection
        # straight into the container's QEMU SLiRP hostfwd, which the
        # 2026-04-30 diagnostic confirmed delivers all the way through
        # to the guest's smoltcp listener.
        & docker run --rm --name $containerName -d -p 127.0.0.1:8080:8080 arest-kernel-server | Out-Null
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    if ($LASTEXITCODE -ne 0) { throw "docker run failed (exit $LASTEXITCODE)" }

    try {
        # Poll on the FINAL banner line entry_uefi writes — "server:
        # net+http loop running" — rather than an early line, so the
        # snapshot includes everything the kernel printed pre-loop.
        $deadline = (Get-Date).AddSeconds(90)
        $log = ""
        $serverLineSeen = $false
        while ((Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 500
            $prevEAP = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $log = (docker logs $containerName 2>&1 | Out-String)
            } finally {
                $ErrorActionPreference = $prevEAP
            }
            if ($log -match "server:\s+net\+http loop running") {
                $serverLineSeen = $true
                break
            }
            # Defensive: if the kernel panics before the server line,
            # surface that early instead of waiting out the cap.
            if ($log -match "UEFI kernel panic") { break }
        }
        $log | Out-File -FilePath $logPath -Encoding utf8

        if (-not $serverLineSeen) {
            Write-Host "FAIL: 'server: net+http loop running' banner not observed within 90 s." -ForegroundColor Red
            Write-Host "`n--- captured serial log ($logPath) ---"
            Write-Host $log
            exit 1
        }

        # Banner phrases that prove every prerequisite subsystem booted.
        # Mirrors `boot-kernel-uefi.ps1`'s assertion set, minus the
        # entries elided on the lean server profile (slint launcher,
        # PS/2 REPL, virtio-gpu / virtio-input — see Dockerfile.uefi-
        # server CMD line).
        $expected = @(
            "AREST kernel - UEFI scaffold (#344)",
            "step 4 of 8: ExitBootServices + post-EBS serial",
            "post-EBS: 16550 COM1 active",
            "frames usable",
            "dma:      pool live",
            "pci:      walk OK (virtio-net:",
            "virtio-net: driver online, MAC",
            "virtio-blk: driver online,",
            "block:    checkpoint round-trip OK",
            "engine:   system::init() completed",
            # #655 prereqs for host-reachable :8080
            "net:      smoltcp interface live",
            "http:     handler registered on :80",
            # The server-profile beacon (#627 Profile-3 banner)
            "server:   net+http loop running"
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

        # Host -> docker:8080 -> QEMU hostfwd -> guest:80 reachability.
        # HARD check (#657 closed): proves the QEMU SLiRP hostfwd
        # reaches the guest's smoltcp listener and the kernel's
        # arest_http_handler returns a real response. Required for
        # the apis e2e suite (#624).
        #
        # Use `127.0.0.1` explicitly rather than `localhost` — on
        # Windows, `localhost` resolves to `::1` first (IPv6), but
        # Docker Desktop's port-mapping only forwards IPv4 by
        # default, so the IPv6 attempt times out with "Empty reply
        # from server". The IPv4 path is the one the smoke is
        # actually testing; pinning the URL to 127.0.0.1 routes
        # around the resolver order. (Diagnosed 2026-04-30 — the
        # original Empty-reply mystery was DNS, not the kernel net
        # stack which had been instrumented with rx/tx counters and
        # observed delivering the full handshake + response wire
        # bytes via the IPv4 path.)
        # Use curl.exe rather than Invoke-WebRequest. IWR's `-Uri`
        # implementation depends on Internet Explorer / Edge COM
        # interfaces that PowerShell 5.1 in `-NonInteractive` mode
        # cannot drive (`PromptForChoice`-class init failures show
        # up as silent "Empty reply from server" timeouts in the
        # smoke runner's captured stderr stream). curl.exe ships
        # with Windows 10+ and 11 by default and has no such
        # dependency; it also gives us a clean exit code + body
        # capture without the elaborate Invoke-WebRequest object
        # surface.
        $curlOk = $false
        $curlBody = $null
        $curlStatus = $null
        $curlBodyFile = New-TemporaryFile
        $curlDeadline = (Get-Date).AddSeconds(60)
        $curlStart = Get-Date
        while ((Get-Date) -lt $curlDeadline) {
            $code = & curl.exe -s -m 3 -o $curlBodyFile.FullName -w "%{http_code}" "http://127.0.0.1:8080/" 2>$null
            if ($LASTEXITCODE -eq 0 -and $code -match '^[0-9]+$' -and [int]$code -ge 200 -and [int]$code -lt 500) {
                $curlOk = $true
                $curlStatus = [int]$code
                $curlBody = Get-Content $curlBodyFile.FullName -Raw -ErrorAction SilentlyContinue
                if ($null -eq $curlBody) { $curlBody = "" }
                break
            }
            Start-Sleep -Milliseconds 500
        }
        $curlElapsed = ((Get-Date) - $curlStart).TotalSeconds
        Remove-Item $curlBodyFile.FullName -Force -ErrorAction SilentlyContinue

        if (-not $curlOk) {
            Write-Host "FAIL: http://127.0.0.1:8080/ unreachable from host within 60 s." -ForegroundColor Red
            Write-Host "      Banner observed but the host-curl path is broken." -ForegroundColor Red
            Write-Host "`n--- captured serial log ($logPath) ---"
            Write-Host $log
            exit 1
        }
        $bodyPreview = if ($curlBody.Length -gt 120) { $curlBody.Substring(0, 120) + "..." } else { $curlBody }
        Write-Host ("PASS: http://127.0.0.1:8080/ reachable in {0:N1} s (HTTP {1}, {2} bytes)." -f $curlElapsed, $curlStatus, $curlBody.Length) -ForegroundColor Green
        Write-Host "      Body preview: $bodyPreview" -ForegroundColor DarkGray

        Write-Host "PASS: server-profile UEFI banner observed end-to-end." -ForegroundColor Green
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

Write-Host "`nBooting UEFI kernel server under QEMU + OVMF (Docker)..." -ForegroundColor Cyan
Write-Host "Ctrl-C here to stop the kernel." -ForegroundColor DarkGray
Write-Host "Note: OVMF prints its own boot banners before ours; AREST output appears after.`n" -ForegroundColor DarkGray
docker run --rm -p 8080:8080 arest-kernel-server
