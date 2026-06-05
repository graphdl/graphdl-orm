# scripts/boot-uefi-window.ps1
#
# Boot the current default-profile arest-kernel .efi in an interactive
# QEMU SDL window (host-QEMU, no Docker) for hands-on UI testing — the
# host keyboard + mouse reach the kernel's PS/2 drivers (IRQ 1 / IRQ 12).
# Serial streams to target/probe-uefi/serial.log; after the window
# closes this greps it for the pointer-ring debug output so a mouse
# test self-reports PASS/empty without any scripted input injection
# (which an AV's ClickFix heuristic flags — #598).
#
# Assumes the .efi is already built (e.g. the default-profile
# `cargo +nightly-2026-04-21 build --target x86_64-unknown-uefi
# --release` from crates/arest-kernel).
#
# Usage:  pwsh scripts\boot-uefi-window.ps1

$ErrorActionPreference = "Stop"
$repoRoot  = (Resolve-Path "$PSScriptRoot\..").Path
$efi       = Join-Path $repoRoot "crates\arest-kernel\target\x86_64-unknown-uefi\release\arest-kernel.efi"
if (-not (Test-Path $efi)) {
    throw "no .efi at $efi — build the default profile first (cargo +nightly-2026-04-21 build --target x86_64-unknown-uefi --release from crates\arest-kernel)"
}

$stageAbs  = Join-Path $repoRoot "target\probe-uefi"
$espAbs    = Join-Path $stageAbs "esp"
$serialAbs = Join-Path $stageAbs "serial.log"
$stageRel  = "target/probe-uefi"
$espRel    = "$stageRel/esp"
$qemu      = "C:\Program Files\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) {
    $c = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
    if ($c) { $qemu = $c.Source } else { throw "qemu-system-x86_64 not found" }
}
$share = Join-Path (Split-Path $qemu -Parent) "share"

New-Item -ItemType Directory -Force -Path (Join-Path $espAbs "EFI\BOOT") | Out-Null
Copy-Item $efi (Join-Path $espAbs "EFI\BOOT\BOOTX64.EFI") -Force
Copy-Item (Join-Path $share "edk2-x86_64-code.fd") (Join-Path $stageAbs "ovmf-code.fd") -Force
Copy-Item (Join-Path $share "edk2-i386-vars.fd")  (Join-Path $stageAbs "ovmf-vars.fd")  -Force
$disk = Join-Path $stageAbs "disk.img"
if (-not (Test-Path $disk)) { fsutil file createnew $disk 10485760 | Out-Null }
if (Test-Path $serialAbs) { Clear-Content $serialAbs }

Write-Host "Booting default-profile kernel in a QEMU SDL window." -ForegroundColor Green
Write-Host "  -> Move the mouse over the window: the cursor sprite should track it." -ForegroundColor Green
Write-Host "  -> Click around: clicks dispatch into the Slint launcher." -ForegroundColor Green
Write-Host "  -> Close the window to quit; this then reports the captured pointer events.`n" -ForegroundColor Green

$qargs = @(
    "-drive", "if=pflash,format=raw,unit=0,readonly=on,file=$stageRel/ovmf-code.fd",
    "-drive", "if=pflash,format=raw,unit=1,file=$stageRel/ovmf-vars.fd",
    "-m", "1024",
    "-drive", "file=fat:rw:$espRel,if=ide",
    "-device", "virtio-gpu-pci",
    "-chardev", "file,id=ser0,path=$serialAbs",
    "-serial", "chardev:ser0",
    "-display", "sdl", "-no-reboot", "-no-shutdown"
)

Push-Location $repoRoot
try { & $qemu @qargs } finally { Pop-Location }

$ptr = Get-Content $serialAbs -ErrorAction SilentlyContinue | Select-String -Pattern "ptr-dbg:"
if ($ptr) {
    Write-Host "`nPASS: pointer events reached the kernel ring ($($ptr.Count) ptr-dbg lines). First few:" -ForegroundColor Green
    $ptr | Select-Object -First 12 | ForEach-Object { Write-Host "  $($_.Line.Trim())" }
} else {
    Write-Host "`nNo 'ptr-dbg:' lines captured — was the mouse moved over the window?" -ForegroundColor Yellow
    Write-Host "Full serial: $serialAbs"
}
