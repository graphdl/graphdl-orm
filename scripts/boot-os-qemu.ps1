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
  [switch]$Release,
  [string]$Features = '',
  [int]$TimeoutSec = 45,
  [switch]$Keep
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$osDir = Join-Path $repoRoot 'engine\os'
$profileDir = if ($Release) { 'release' } else { 'debug' }
$efi = Join-Path $osDir ("target\x86_64-unknown-uefi\$profileDir\arest-os.efi")

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
    if ($Release) { $args += '--release' }
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
# BDS sometimes drops to the UEFI shell instead of probing the
# removable-media default path; startup.nsh makes the shell run the
# loader itself after its countdown, so both boot paths converge.
Set-Content -Path (Join-Path $work 'esp\startup.nsh') -Value 'FS0:\EFI\BOOT\BOOTX64.EFI' -Encoding ascii
$code = Join-Path $work 'code.fd'; Copy-Item $codeSrc $code
$vars = Join-Path $work 'vars.fd'; Copy-Item $varsSrc $vars
$serial = Join-Path $work 'serial.log'

# --- boot --------------------------------------------------------------
$qargs = @(
  '-machine', 'q35', '-m', '256M', '-display', 'none',
  # TCG on purpose: WHPX + OVMF + -cpu max faults in PlatformPei
  # (tried 2026-07-08). The boot path must stay cheap enough for pure
  # emulation — the interpretive verbs are NOT (the engine's own note:
  # minutes at tasks scale); boot probes ride the native carrier.
  # std's UEFI random source needs EFI_RNG_PROTOCOL: virtio-rng feeds
  # OVMF's VirtioRngDxe, and -cpu max adds RDRAND for RngDxe's CPU path
  '-cpu', 'max', '-device', 'virtio-rng-pci',
  # the wire: OVMF's VirtioNetDxe turns this NIC into an SNP handle;
  # hostfwd lets the smoke curl the verb table on bare firmware
  '-device', 'virtio-net-pci,netdev=n0',
  '-netdev', 'user,id=n0,hostfwd=tcp:127.0.0.1:18080-:80',
  # the full target's framebuffer: OVMF's virtio-gpu driver exposes
  # GOP even headless (-display none just doesn't show it)
  '-device', 'virtio-gpu-pci',
  '-drive', "if=pflash,format=raw,readonly=on,file=$code",
  '-drive', "if=pflash,format=raw,file=$vars",
  '-drive', ("format=raw,file=fat:rw:" + (Join-Path $work 'esp')),
  '-serial', "file:$serial",
  '-no-reboot'
)
$p = Start-Process -FilePath $qemu -ArgumentList $qargs -PassThru -WindowStyle Hidden
$deadline = (Get-Date).AddSeconds($TimeoutSec)
$phrases = @('AREST OS', 'boot: complete')   # version-agnostic on purpose
$pass = $false
$wirePass = $false
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Seconds 2
  if (Test-Path $serial) {
    $txt = Get-Content $serial -Raw -ErrorAction SilentlyContinue
    if ($txt -and ($phrases | Where-Object { $txt -notmatch [regex]::Escape($_) }).Count -eq 0) {
      $pass = $true
      # the wire assertion: once the banner stands, curl the verb
      # table through the hostfwd — the engine answering HTTP from
      # bare firmware is the server target's proof
      if ($txt -match 'wire: listening') {
        try {
          $r = Invoke-WebRequest -Uri 'http://127.0.0.1:18080/version' -TimeoutSec 10 -UseBasicParsing
          if ($r.Content -match 'AREST OS') {
            Write-Host ("wire answer: " + $r.Content)
            # a STORE verb over the wire: the native get, same as the
            # Worker serves — the engine surface, headless
            $g = Invoke-WebRequest -Uri 'http://127.0.0.1:18080/get?args=%7B%22noun%22%3A%22Contact%20Submission%22%2C%22id%22%3A%22ef998c6716463931%22%7D' -TimeoutSec 10 -UseBasicParsing
            if ($g.Content -match '"exists"') {
              $wirePass = $true
              Write-Host ("wire get: " + $g.Content.Substring(0, [Math]::Min(140, $g.Content.Length)))
            }
          }
        } catch { Write-Host "wire curl failed: $_" }
      }
      break
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
  if ($pass -and $wirePass) { Write-Host 'SMOKE PASS (wire)'; exit 0 }
  if ($pass) { Write-Host 'SMOKE PASS (banner only)'; exit 0 }
  Write-Host 'SMOKE FAIL'; exit 1
}
