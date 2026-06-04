# scripts/boot-kernel-uefi-mini.ps1
#
# Profile-6b mini smoke harness (#631) — HOST-QEMU edition.
#
# Unlike the Docker boot-kernel-uefi*.ps1 scripts, this builds and boots
# entirely on the host (no Docker): host `cargo` cross-compiles the kernel
# with `--no-default-features --features mini,static-ip` to
# x86_64-unknown-uefi, then host QEMU boots it under OVMF (pflash split)
# and asserts the mini boot banner + a host GET on :8080.
#
# `mini = server + slint + ui-bundle` (slint pulls repl), so the boot runs
# the full kernel bring-up — UI launcher, REPL, engine, wasmi — AND the
# headless server net+http loop. It compiles WITHOUT linuxkpi/wine/doom,
# so the banner carries NO `virtio-input:` lines (asserted absent below)
# and the doom GAME is not built (the `doom: skipped` line proves it).
# The smoke fails if any of those layers sneak back in.
#
# Usage:
#   .\scripts\boot-kernel-uefi-mini.ps1            # build + headless smoke
#   .\scripts\boot-kernel-uefi-mini.ps1 -Rebuild   # force recompile
#   .\scripts\boot-kernel-uefi-mini.ps1 -Window    # interactive QEMU (SDL)
#
# Requires: rustup nightly-2026-04-21 + x86_64-unknown-uefi + rust-src;
# QEMU for Windows (qemu-system-x86_64 + share\edk2-x86_64-code.fd +
# share\edk2-i386-vars.fd). QEMU file args are kept RELATIVE to the repo
# root (QEMU's working dir) so the VVFAT `fat:rw:` spec doesn't choke on a
# Windows drive-letter colon; OVMF ships as split pflash images that can't
# be loaded via -bios, hence the pflash pair.

param(
    [switch]$Rebuild,
    [switch]$Window
)

$ErrorActionPreference = "Stop"
$repoRoot  = (Resolve-Path "$PSScriptRoot\..").Path
$kernelDir = Join-Path $repoRoot "crates\arest-kernel"
$efi       = Join-Path $kernelDir "target\x86_64-unknown-uefi\release\arest-kernel.efi"

$stageRel  = "target/mini-uefi"
$stageAbs  = Join-Path $repoRoot "target\mini-uefi"
$espRel    = "$stageRel/esp"
$espAbs    = Join-Path $stageAbs "esp"
$serialAbs = Join-Path $stageAbs "serial.log"

$qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) {
    $c = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
    if ($c) { $qemu = $c.Source } else { throw "qemu-system-x86_64 not found. Install QEMU for Windows (https://qemu.weilnetz.de/w64/)." }
}
$share = Join-Path (Split-Path $qemu -Parent) "share"

# ── 1. Build the mini .efi on the host ─────────────────────────────────
if ($Rebuild -or -not (Test-Path $efi)) {
    Write-Host "Building mini .efi (host cargo --features mini,static-ip)..." -ForegroundColor Cyan
    Push-Location $kernelDir
    $prevEAP = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    $prevRF = $env:RUSTFLAGS; $env:RUSTFLAGS = "--cfg poly1305_force_soft"
    try {
        cargo +nightly-2026-04-21 build --target x86_64-unknown-uefi --release --no-default-features --features mini,static-ip
    } finally { $env:RUSTFLAGS = $prevRF; $ErrorActionPreference = $prevEAP; Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
} else {
    Write-Host "Reusing existing .efi (pass -Rebuild to recompile)." -ForegroundColor DarkGray
}
if (-not (Test-Path $efi)) { throw "no .efi produced at $efi" }

# ── 2. Stage ESP + OVMF (pflash) + virtio-blk disk ─────────────────────
New-Item -ItemType Directory -Force -Path (Join-Path $espAbs "EFI\BOOT") | Out-Null
Copy-Item $efi (Join-Path $espAbs "EFI\BOOT\BOOTX64.EFI") -Force
Copy-Item (Join-Path $share "edk2-x86_64-code.fd") (Join-Path $stageAbs "ovmf-code.fd") -Force
Copy-Item (Join-Path $share "edk2-i386-vars.fd")  (Join-Path $stageAbs "ovmf-vars.fd")  -Force  # writable vars
$disk = Join-Path $stageAbs "disk.img"
if (-not (Test-Path $disk)) { fsutil file createnew $disk 10485760 | Out-Null }

# ── 3. QEMU args ───────────────────────────────────────────────────────
# Mini drops virtio-keyboard/tablet (no linuxkpi -> no virtio-input driver).
$display = if ($Window) { @("-display", "sdl") } else { @("-display", "none") }
$qargs = @(
    "-drive", "if=pflash,format=raw,unit=0,readonly=on,file=$stageRel/ovmf-code.fd",
    "-drive", "if=pflash,format=raw,unit=1,file=$stageRel/ovmf-vars.fd",
    "-m", "1024",
    "-drive", "file=fat:rw:$espRel,if=ide",
    "-netdev", "user,id=net0,hostfwd=tcp::8080-:80",
    "-device", "virtio-net-pci,netdev=net0,disable-legacy=on",
    "-drive", "file=$stageRel/disk.img,format=raw,if=none,id=disk0",
    "-device", "virtio-blk-pci,drive=disk0,disable-legacy=on",
    "-device", "virtio-gpu-pci",
    "-chardev", "file,id=ser0,path=$serialAbs",
    "-serial", "chardev:ser0"
) + $display + @("-no-reboot", "-no-shutdown")

if ($Window) {
    Write-Host "Launching interactive QEMU window (close it to quit)..." -ForegroundColor Green
    & $qemu @qargs
    return
}

# ── 4. Headless smoke: boot, assert banner, soft-curl :8080 ────────────
Write-Host "Booting mini kernel headless (host QEMU + OVMF, ~TCG)..." -ForegroundColor Cyan
if (Test-Path $serialAbs) { Clear-Content $serialAbs -ErrorAction SilentlyContinue }
$p = Start-Process -FilePath $qemu -ArgumentList $qargs -PassThru -NoNewWindow -WorkingDirectory $repoRoot
$exitCode = 0
try {
    # Poll the serial log for the server beacon (last line before the
    # net+http drainer) or a panic, capped at 120 s (host TCG is slow).
    $deadline = (Get-Date).AddSeconds(120)
    $beaconSeen = $false
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 750
        $log = (Get-Content $serialAbs -Raw -ErrorAction SilentlyContinue)
        if ($null -eq $log) { continue }
        if ($log -match "server:\s+net\+http loop running") { $beaconSeen = $true; break }
        if ($log -match "UEFI kernel panic") { break }
    }
    $log = (Get-Content $serialAbs -Raw -ErrorAction SilentlyContinue)
    if ($null -eq $log) { $log = "" }

    if (-not $beaconSeen) {
        Write-Host "FAIL: 'server: net+http loop running' beacon not seen within 120 s." -ForegroundColor Red
        Write-Host "`n--- serial ($serialAbs) ---`n$log"
        $exitCode = 1
    } else {
        # Banner phrases — the REAL mini banner captured on #631's first
        # host boot. Mini = full bring-up + server beacon, minus the
        # linuxkpi virtio-input lines.
        $expected = @(
            "AREST kernel - UEFI scaffold (#344)",
            "step 4 of 8: ExitBootServices + post-EBS serial",
            "post-EBS: 16550 COM1 active",
            "gate:     ring-3 userspace gate online",
            "entropy:  x86_64 hardware RNG",
            "pit:      1 kHz timer online",
            "kbd:      PS/2 driver online",
            "frames usable",
            "idt:      int3 round-tripped through UEFI IDT",
            "dma:      pool live",
            "pci:      walk OK (virtio-net:",
            "virtio-net: driver online, MAC",
            "net:      smoltcp interface live",
            "http:     handler registered on :80",
            "virtio-blk: driver online,",
            "block:    checkpoint round-trip OK",
            "gop:      ",
            "fb:       paint smoke OK",
            "virtio-gpu: driver online,",
            "fb:       virtio-gpu surface installed",
            "engine:   system::init() completed (arest engine live on UEFI)",
            "wasmi:    tiny module executed, main() = 42",
            "doom:     skipped (build without --features doom",
            "repl:     line-buffered keyboard REPL online",
            "ui:       launcher running",
            "server:   net+http loop running"
        )
        $missing = @()
        foreach ($phrase in $expected) { if ($log -notmatch [regex]::Escape($phrase)) { $missing += $phrase } }

        # Guard (#631's whole point): mini must NOT pull linuxkpi, so no
        # virtio-input driver lines may appear.
        $leaked = @()
        foreach ($phrase in @("virtio-input:")) { if ($log -match [regex]::Escape($phrase)) { $leaked += $phrase } }

        if ($missing.Count -gt 0 -or $leaked.Count -gt 0) {
            if ($missing.Count -gt 0) { Write-Host "FAIL: missing banner phrases:" -ForegroundColor Red; $missing | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red } }
            if ($leaked.Count -gt 0)  { Write-Host "FAIL: mini leaked a layer it must not build (linuxkpi/virtio-input):" -ForegroundColor Red; $leaked | ForEach-Object { Write-Host "  + $_" -ForegroundColor Red } }
            Write-Host "`n--- serial ($serialAbs) ---`n$log"
            $exitCode = 1
        } else {
            Write-Host "PASS: mini banner observed (full bring-up + server beacon; no linuxkpi/doom)." -ForegroundColor Green

            # Soft host curl — matches the full smoke (boot-kernel-uefi.ps1):
            # banner-level PASS stands on its own; the host-reachable half is
            # bonus (SLiRP DHCPv4 settle can lag past the window).
            $curlOk = $false; $code = $null
            $deadlineC = (Get-Date).AddSeconds(45)
            while ((Get-Date) -lt $deadlineC) {
                $code = (& curl.exe -s -m 3 -o NUL -w "%{http_code}" "http://127.0.0.1:8080/" 2>$null)
                if ($LASTEXITCODE -eq 0 -and $code -match '^[0-9]+$' -and [int]$code -ge 200 -and [int]$code -lt 500) { $curlOk = $true; break }
                Start-Sleep -Milliseconds 750
            }
            if ($curlOk) { Write-Host "PASS: http://127.0.0.1:8080/ reachable (HTTP $code) — host curl path verified." -ForegroundColor Green }
            else { Write-Host "WARN: :8080 not reachable from host within 45 s (banner smoke still PASSES; the net path is the soft half, as in the full smoke)." -ForegroundColor Yellow }
        }
    }
} finally {
    if (-not $p.HasExited) { $p.Kill() }
}

if ($exitCode -eq 0) {
    Write-Host "`nPASS: mini UEFI smoke green." -ForegroundColor Green
    Write-Host "Serial log: $serialAbs"
}
exit $exitCode
