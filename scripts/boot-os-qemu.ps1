<#
.SYNOPSIS
  UEFI smoke harness for engine/os: build arest-os.efi, boot it under
  host QEMU + split OVMF headless, assert the boot banner from the
  serial capture (ConOut is mirrored to COM1 by OVMF).

.DESCRIPTION
  Mechanics inherited from the pre-0.9.0 harness (66a580ab):
    * The .efi stages at <esp>\EFI\BOOT\BOOTX64.EFI — the removable-
      media default path OVMF probes when no boot var names a loader.
    * Firmware is the split pair edk2-x86_64-code.fd (read-only pflash
      unit 0) + a writable copy of edk2-i386-vars.fd (unit 1).
    * QEMU's -drive option-string splits values at spaces, so firmware
      + ESP stage in a space-free temp dir (never "C:\Program Files").

.PARAMETER Smoke
  Headless: cap the run, assert the banner phrases, exit 0/1.

.PARAMETER SkipBuild
  Reuse target\x86_64-unknown-uefi\debug\arest-os.efi.

.PARAMETER Features
  Cargo features for the build (default: none -> server target).

.PARAMETER TimeoutSec
  Hard cap on the QEMU run (default 45); the OS idles after boot.
#>
[CmdletBinding()]
param(
  [switch]$Smoke,
  [switch]$SkipBuild,
  [string]$Features = '',
  [int]$TimeoutSec = 45,
  [switch]$Keep
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$osDir = Join-Path $repoRoot 'engine\os'
$efi = Join-Path $osDir 'target\x86_64-unknown-uefi\debug\arest-os.efi'

# --- locate QEMU + OVMF ------------------------------------------------
$qemu = (Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue).Source
if (-not $qemu) {
  $cand = 'C:\Program Files\qemu\qemu-system-x86_64.exe'
  if (Test-Path $cand) { $qemu = $cand }
}
if (-not $qemu) { throw 'qemu-system-x86_64 not found (PATH, C:\Program Files\qemu).' }
$share = Join-Path (Split-Path $qemu -Parent) 'share'
$codeSrc = Join-Path $share 'edk2-x86_64-code.fd'
$varsSrc = Join-Path $share 'edk2-i386-vars.fd'
foreach ($f in @($codeSrc, $varsSrc)) {
  if (-not (Test-Path $f)) { throw "OVMF firmware not found: $f" }
}

# --- build -------------------------------------------------------------
if (-not $SkipBuild) {
  Push-Location $osDir
  try {
    $args = @('+nightly', 'build', '--target', 'x86_64-unknown-uefi')
    if ($Features) { $args += @('--features', $Features) }
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
  } finally { Pop-Location }
}
if (-not (Test-Path $efi)) { throw "missing artifact: $efi" }

# --- stage the ESP + firmware in a space-free dir ----------------------
$work = Join-Path $env:TEMP ("arest-os-smoke-" + [IO.Path]::GetRandomFileName().Replace('.', ''))
$espDir = Join-Path $work 'esp\EFI\BOOT'
New-Item -ItemType Directory -Force $espDir | Out-Null
Copy-Item $efi (Join-Path $espDir 'BOOTX64.EFI')
$code = Join-Path $work 'code.fd'; Copy-Item $codeSrc $code
$vars = Join-Path $work 'vars.fd'; Copy-Item $varsSrc $vars
$serial = Join-Path $work 'serial.log'

# --- boot --------------------------------------------------------------
$qargs = @(
  '-machine', 'q35', '-m', '256M', '-display', 'none',
  '-drive', "if=pflash,format=raw,readonly=on,file=$code",
  '-drive', "if=pflash,format=raw,file=$vars",
  '-drive', ("format=raw,file=fat:rw:" + (Join-Path $work 'esp')),
  '-serial', "file:$serial",
  '-no-reboot'
)
$p = Start-Process -FilePath $qemu -ArgumentList $qargs -PassThru -WindowStyle Hidden
$deadline = (Get-Date).AddSeconds($TimeoutSec)
$phrases = @('AREST OS 0.9.0', 'boot: complete')
$pass = $false
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Seconds 2
  if (Test-Path $serial) {
    $txt = Get-Content $serial -Raw -ErrorAction SilentlyContinue
    if ($txt -and ($phrases | Where-Object { $txt -notmatch [regex]::Escape($_) }).Count -eq 0) {
      $pass = $true; break
    }
  }
  if ($p.HasExited) { break }
}
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -Confirm:$false }

$txt = if (Test-Path $serial) { Get-Content $serial -Raw } else { '' }
Write-Host '--- serial tail ---'
($txt -split "`n" | Select-Object -Last 12) | Write-Host
if (-not $Keep) { Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue }

if ($Smoke) {
  if ($pass) { Write-Host 'SMOKE PASS'; exit 0 }
  Write-Host 'SMOKE FAIL'; exit 1
}
