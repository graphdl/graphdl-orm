# scripts/boot-kernel-uefi-graphical.ps1
#
# Interactive "play DOOM on AREST" path. Builds (or reuses) the
# doom-enabled UEFI kernel image (Dockerfile.uefi-graphical,
# --features doom,linuxkpi), extracts the boot artifacts, and launches
# them in a NATIVE QEMU window with a Slint display + virtio input.
#
# #595: doom is re-enabled here -- the tickGame OOB that used to disable
# it (and that the stale comments blamed for a kernel #DF) is fixed by
# the 64 MiB doom heap (commit b771feea). The headless Docker boot
# (boot-kernel-uefi*.ps1) runs under TCG software emulation and is very
# slow; this native path uses WHPX (Windows Hypervisor) so the guest --
# and thus the wasmi interpreter running Doom -- runs near-native.
#
# Usage from repo root:
#   .\scripts\boot-kernel-uefi-graphical.ps1            # WHPX (fast)
#   .\scripts\boot-kernel-uefi-graphical.ps1 -NoAccel   # TCG (slow, but
#                                                         works if WHPX
#                                                         errors on your box)
#   .\scripts\boot-kernel-uefi-graphical.ps1 -Rebuild   # force image rebuild
#
# Controls: the SDL window has keyboard focus; play Doom directly.
# Close the window (or Ctrl-C in this console) to quit. The kernel serial
# (boot banners + `doom:` logs) streams to this console.

param(
    [switch]$Rebuild,
    [switch]$NoAccel
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$img = "arest-kernel-uefi-graphical"

$qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) {
    $cmd = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
    if ($cmd) { $qemu = $cmd.Source }
    else { throw "qemu-system-x86_64 not found. Install QEMU for Windows (https://qemu.weilnetz.de/w64/)." }
}

# ── 1. Build (or reuse cached image) ───────────────────────────────────
$haveImage = [bool](docker images -q $img 2>$null)
if ($Rebuild -or -not $haveImage) {
    Write-Host "Building $img (doom,linuxkpi) — this is the slow step (~min)..." -ForegroundColor Cyan
    $prevEAP = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try {
        docker build -t $img -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi-graphical" $repoRoot
    } finally { $ErrorActionPreference = $prevEAP }
    if ($LASTEXITCODE -ne 0) { throw "docker build failed (exit $LASTEXITCODE)" }
} else {
    Write-Host "Reusing cached image $img (pass -Rebuild to rebuild)." -ForegroundColor DarkGray
}

# ── 2. Extract boot artifacts from the image ───────────────────────────
$playDir = Join-Path $repoRoot "target\doom-play"
New-Item -ItemType Directory -Force -Path $playDir | Out-Null
$cn = "doom-extract-$([guid]::NewGuid().ToString('N').Substring(0,8))"
$prevEAP = $ErrorActionPreference; $ErrorActionPreference = "Continue"
try {
    docker create --name $cn $img | Out-Null
    docker cp "${cn}:/uefi-disk.img" "$playDir\uefi-disk.img"
    docker cp "${cn}:/disk.img"      "$playDir\disk.img"
    docker cp "${cn}:/usr/share/ovmf/OVMF.fd" "$playDir\OVMF.fd"
} finally {
    docker rm -f $cn 2>$null | Out-Null
    $ErrorActionPreference = $prevEAP
}
Write-Host "Extracted boot artifacts to $playDir" -ForegroundColor DarkGray

# ── 3. Launch native QEMU with a graphical window ──────────────────────
# WHPX makes the guest CPU run near-native (TCG emulates it in software,
# which is what makes the headless Docker boot crawl). `kernel-irqchip=off`
# is the form that works with QEMU+WHPX on most Windows hosts.
$accel = if ($NoAccel) { @() } else { @("-accel", "whpx,kernel-irqchip=off") }
if ($NoAccel) {
    Write-Host "Running under TCG (no acceleration) — Doom will be slow." -ForegroundColor Yellow
} else {
    Write-Host "Running with WHPX acceleration. If QEMU errors immediately, re-run with -NoAccel." -ForegroundColor Green
}
Write-Host "Launching DOOM on AREST. Close the QEMU window (or Ctrl-C here) to quit.`n" -ForegroundColor Green

& $qemu `
    -bios "$playDir\OVMF.fd" `
    -m 1024 `
    @accel `
    -drive "format=raw,file=$playDir\uefi-disk.img,if=ide" `
    -netdev "user,id=net0,hostfwd=tcp::8080-:80" `
    -device "virtio-net-pci,netdev=net0,disable-legacy=on" `
    -drive "file=$playDir\disk.img,format=raw,if=none,id=disk0" `
    -device "virtio-blk-pci,drive=disk0,disable-legacy=on" `
    -device "virtio-gpu-pci" `
    -device "virtio-keyboard-pci" `
    -device "virtio-tablet-pci" `
    -serial stdio `
    -display sdl `
    -no-reboot -no-shutdown
