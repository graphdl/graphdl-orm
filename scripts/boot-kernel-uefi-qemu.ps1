<#
.SYNOPSIS
  Docker-free UEFI smoke harness: build arest-kernel.efi locally, boot it
  under QEMU + split OVMF firmware headless, and assert the boot-banner
  stack the kernel writes through ConOut (mirrored to COM1).

.DESCRIPTION
  The original smoke path (scripts/boot-kernel-uefi.ps1) builds + boots
  inside Docker. This script needs no Docker — it uses the host QEMU
  (qemu-system-x86_64) + the edk2 OVMF firmware that ships with it, and a
  QEMU virtual-FAT ESP, so it runs anywhere the QEMU MSI is installed.

  Boot mechanics:
    * The kernel .efi is staged at <esp>\EFI\BOOT\BOOTX64.EFI — the
      removable-media default path OVMF probes when no boot var names a
      loader.
    * Firmware is the split pair edk2-x86_64-code.fd (read-only, pflash
      unit 0) + a writable copy of edk2-i386-vars.fd (pflash unit 1).
    * QEMU's -drive option-string splits values at spaces and
      Start-Process won't quote them, so the firmware + ESP are staged in
      a space-free temp dir (NOT "C:\Program Files\...").
    * AREST writes to ConOut; OVMF mirrors ConOut to the 16550 COM1, so
      -serial file:<log> captures the full banner.

.PARAMETER Smoke
  Headless: cap the run, assert the banner phrase set, exit 0 (PASS) /
  1 (FAIL). Without it, the kernel boots and the serial log streams until
  the timeout (handy for eyeballing a boot).

.PARAMETER SkipBuild
  Reuse the existing target\x86_64-unknown-uefi\debug\arest-kernel.efi
  instead of running cargo build first.

.PARAMETER TimeoutSec
  Hard cap on the QEMU run (default 60). The kernel idles forever after
  boot, so the harness always kills QEMU at this deadline.

.PARAMETER Keep
  Leave the staging workdir (ESP, firmware copies, serial.log) in place
  for inspection instead of reporting just the log tail.
#>
[CmdletBinding()]
param(
  [switch]$Smoke,
  [switch]$SkipBuild,
  [int]$TimeoutSec = 60,
  [switch]$Keep
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$kernelDir = Join-Path $repoRoot 'crates\arest-kernel'
$efi = Join-Path $kernelDir 'target\x86_64-unknown-uefi\debug\arest-kernel.efi'

# --- locate QEMU + OVMF firmware -------------------------------------
$qemu = (Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue).Source
if (-not $qemu) {
  $cand = 'C:\Program Files\qemu\qemu-system-x86_64.exe'
  if (Test-Path $cand) { $qemu = $cand }
}
if (-not $qemu) { throw "qemu-system-x86_64 not found (looked on PATH and C:\Program Files\qemu)." }
$share = Join-Path (Split-Path $qemu -Parent) 'share'
$codeSrc = Join-Path $share 'edk2-x86_64-code.fd'
$varsSrc = Join-Path $share 'edk2-i386-vars.fd'
foreach ($f in @($codeSrc, $varsSrc)) {
  if (-not (Test-Path $f)) { throw "OVMF firmware not found: $f" }
}

# --- build (unless skipped) ------------------------------------------
if (-not $SkipBuild) {
  Write-Host "Building arest-kernel.efi (cargo +nightly build --target x86_64-unknown-uefi)..." -ForegroundColor Cyan
  Push-Location $kernelDir
  $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  try { & cargo +nightly build --target x86_64-unknown-uefi } finally { $ErrorActionPreference = $prev; Pop-Location }
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
}
if (-not (Test-Path $efi)) { throw "kernel .efi missing: $efi (drop -SkipBuild to build it)." }

# --- stage a space-free workdir: ESP + firmware ----------------------
$wd  = Join-Path $env:TEMP 'arest-kernel-qemu'
$esp = Join-Path $wd 'esp'
Remove-Item -Recurse -Force $wd -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $esp 'EFI\BOOT') | Out-Null
Copy-Item $efi (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI') -Force
Copy-Item $codeSrc (Join-Path $wd 'code.fd') -Force
Copy-Item $varsSrc (Join-Path $wd 'vars.fd') -Force
$code = Join-Path $wd 'code.fd'
$vars = Join-Path $wd 'vars.fd'
$serial = Join-Path $wd 'serial.log'
# Blank disk so virtio-blk has a backing store to probe.
$disk = Join-Path $wd 'disk.img'
$fs = [System.IO.File]::Create($disk); $fs.SetLength(16MB); $fs.Close()

$qemuArgs = @(
  '-machine','q35',
  '-m','512',
  '-drive', "if=pflash,format=raw,unit=0,readonly=on,file=$code",
  '-drive', "if=pflash,format=raw,unit=1,file=$vars",
  '-drive', "file=fat:rw:$esp,format=raw,if=ide",
  '-netdev','user,id=net0',
  '-device','virtio-net-pci,netdev=net0,disable-legacy=on',
  '-drive', "file=$disk,format=raw,if=none,id=disk0",
  '-device','virtio-blk-pci,drive=disk0,disable-legacy=on',
  '-device','virtio-gpu-pci',
  '-device','virtio-keyboard-pci',
  '-device','virtio-tablet-pci',
  '-serial', "file:$serial",
  '-display','none','-no-reboot','-no-shutdown'
)

# Banner phrases the kernel writes pre- + post-ExitBootServices. The
# terminal phrase ("launcher running") is the last line, so its presence
# guarantees the snapshot is complete.
$required = @(
  'AREST kernel - UEFI scaffold (#344)',
  'ring-3 userspace gate online',
  'x86_64 hardware RNG',
  '1 kHz timer online',
  'PS/2 driver online',
  'int3 round-tripped through UEFI IDT',
  'smoltcp interface live',
  'virtio-net: driver online',
  'virtio-blk: driver online',
  'virtio-gpu: driver online',
  'system::init() completed (arest engine live on UEFI)',
  'tiny module executed, main() = 42',
  'line-buffered keyboard REPL online',
  'launcher running'
)

Write-Host "Booting arest-kernel.efi under QEMU + OVMF (no Docker, ${TimeoutSec}s cap)..." -ForegroundColor Cyan
$p = Start-Process -FilePath $qemu -ArgumentList $qemuArgs -PassThru -NoNewWindow
$deadline = (Get-Date).AddSeconds($TimeoutSec)
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Milliseconds 1000
  if ($p.HasExited) { break }
  if (Test-Path $serial) {
    $txt = Get-Content $serial -Raw -ErrorAction SilentlyContinue
    if ($txt -and ($txt -match 'launcher running')) { break }
  }
}
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 500

$log = if (Test-Path $serial) { (Get-Content $serial -Raw) -replace "`r","" } else { '' }

if (-not $Smoke) {
  Write-Host "`n=== serial.log ===" -ForegroundColor DarkGray
  Write-Host $log
  if (-not $Keep) { Write-Host "(staging dir: $wd)" -ForegroundColor DarkGray }
  return
}

# Smoke: assert every required banner phrase.
$missing = @($required | Where-Object { $log -notmatch [regex]::Escape($_) })
if ($missing.Count -eq 0) {
  Write-Host "PASS: all $($required.Count) banner phrases observed; kernel reached the REPL on UEFI (no Docker)." -ForegroundColor Green
  if (-not $Keep) { Remove-Item -Recurse -Force $wd -ErrorAction SilentlyContinue }
  exit 0
} else {
  Write-Host "FAIL: missing banner phrases:" -ForegroundColor Red
  $missing | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
  Write-Host "`n=== serial.log ===" -ForegroundColor DarkGray
  Write-Host $log
  Write-Host "(staging dir kept for inspection: $wd)" -ForegroundColor Yellow
  exit 1
}
