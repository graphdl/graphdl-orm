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

.PARAMETER TypeLine
  After the boot banner completes, inject this line into the kernel's
  REPL as PS/2 keystrokes via QMP send-key (a-z, 0-9, space, slash,
  minus, dot, underscore), terminated with Enter. Typing mode drops the
  virtio-keyboard-pci device: QEMU routes send-key events to it when
  present, but the kernel REPL drains the i8042 PS/2 ring — without the
  drop, injected keys would vanish into the unclaimed virtio device.

.PARAMETER ExpectAfter
  Extra serial-log phrases that must appear AFTER the TypeLine has been
  injected (e.g. the guest program's output). Asserted in -Smoke mode in
  addition to the boot-banner set; in non-smoke mode they only extend
  the post-type wait.

.PARAMETER EfiPath
  Boot this .efi instead of the default debug-profile build product —
  e.g. target\x86_64-unknown-uefi\release\arest-kernel.efi for a
  release-profile run (the Slint software renderer is dramatically
  faster there, which matters under TCG). Implies the caller built it;
  combine with -SkipBuild.

.PARAMETER Features
  Cargo feature list for the build step (e.g.
  "busybox,musl-libc,repl" for the #527 typed-exec smoke — `run echo
  hello` needs the baked busybox ELF + its /bin/busybox seed, which
  only exist under those features). Ignored with -SkipBuild.

.PARAMETER ThenType
  Additional lines to send into the serial console AFTER -TypeLine,
  with a settle pause before each. Once -TypeLine has exec'd a guest
  (e.g. `run sh`), the REPL is gone — these lines feed the GUEST's
  blocking stdin read (#476e): `-TypeLine "run sh" -ThenType "echo
  hi" -ExpectAfter "hi"` drives an interactive ash session.
#>
[CmdletBinding()]
param(
  [switch]$Smoke,
  [switch]$SkipBuild,
  [int]$TimeoutSec = 60,
  [switch]$Keep,
  [string]$TypeLine,
  [string[]]$ThenType = @(),
  [string[]]$ExpectAfter = @(),
  [string]$EfiPath,
  [string]$Features
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$kernelDir = Join-Path $repoRoot 'crates\arest-kernel'
$efi = if ($EfiPath) { $EfiPath } else { Join-Path $kernelDir 'target\x86_64-unknown-uefi\debug\arest-kernel.efi' }

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
  # The busybox/musl bake DEGRADES GRACEFULLY (and the /bin/busybox
  # File-fact seed silently vanishes from the kernel — observed as
  # `exec /bin/busybox failed: FileNotFound` in the #527 smoke) when
  # the cross-toolchain is missing from PATH. Non-interactive shells
  # often lack the user's PATH entries, so prepend the known host
  # locations when the tools aren't already visible:
  #   * clang — target compiles (musl objects, busybox objects).
  #   * MinGW gcc — busybox HOST tools (applet_tables/usage); its bin
  #     dir must be on PATH for the runtime DLLs even though build.rs
  #     finds gcc.exe by absolute path.
  if (-not (Get-Command clang -ErrorAction SilentlyContinue)) {
    $llvm = 'C:\Program Files\Microsoft Visual Studio\18\Professional\VC\Tools\Llvm\x64\bin'
    if (Test-Path (Join-Path $llvm 'clang.exe')) { $env:PATH = "$llvm;$env:PATH" }
  }
  $mingw = 'C:\ProgramData\mingw64\mingw64\bin'
  if ((Test-Path (Join-Path $mingw 'gcc.exe')) -and ($env:PATH -notlike "*$mingw*")) {
    $env:PATH = "$mingw;$env:PATH"
  }
  $featureArgs = @()
  if ($Features) { $featureArgs = @('--features', $Features) }
  Write-Host "Building arest-kernel.efi (cargo +nightly build --target x86_64-unknown-uefi $($featureArgs -join ' '))..." -ForegroundColor Cyan
  Push-Location $kernelDir
  $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  try { & cargo +nightly build --target x86_64-unknown-uefi @featureArgs } finally { $ErrorActionPreference = $prev; Pop-Location }
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
  # `-cpu max`: TCG's fullest feature set, including functional RDRAND /
  # RDSEED. The default qemu64 model leaves them absent/exhausted, which
  # boots fine (the entropy probe passes via the boot-time path) but
  # panics the csprng reseed the first time a spawn's AT_RANDOM fill
  # draws entropy (HardwareUnavailable at csprng reseed).
  '-cpu','max',
  '-m','512',
  '-drive', "if=pflash,format=raw,unit=0,readonly=on,file=$code",
  '-drive', "if=pflash,format=raw,unit=1,file=$vars",
  '-drive', "file=fat:rw:$esp,format=raw,if=ide",
  '-netdev','user,id=net0',
  '-device','virtio-net-pci,netdev=net0,disable-legacy=on',
  '-drive', "file=$disk,format=raw,if=none,id=disk0",
  '-device','virtio-blk-pci,drive=disk0,disable-legacy=on',
  '-device','virtio-gpu-pci'
)
$qmpPort = 4444
if ($TypeLine) {
  # Typing mode: QMP socket for send-key. The virtio-keyboard stays
  # attached — QEMU routes send-key events to it as the preferred
  # keyboard handler, and the kernel's linuxkpi virtio-input driver
  # (build with `--features linuxkpi`) translates the EV_KEY events
  # into the keyboard ring. (Empirically on the bundled QEMU dev
  # build + `-display none`, send-key events never reach the i8042:
  # the guest-side PS/2 poll sees an empty output buffer and the
  # i8259 never latches IRQ 1, so the PS/2 route is a dead end for
  # headless typing.)
  $qemuArgs += @('-qmp', "tcp:127.0.0.1:${qmpPort},server,nowait")
}
$serialPort = 4448
$qemuArgs += @(
  '-device','virtio-keyboard-pci,id=vkbd',
  '-device','virtio-tablet-pci'
)
if ($TypeLine) {
  # Typing mode: the serial console is BIDIRECTIONAL over TCP. The
  # harness pumps socket→serial.log (so every assert below reads the
  # same file as the file: path) and writes the TypeLine + CR into the
  # socket; the kernel's super-loop polls COM1 RX and feeds received
  # characters onto the keyboard ring (`keyboard::poll_serial_rx`).
  # This bypasses QEMU's emulated-input layer entirely — on the
  # bundled dev build, headless send-key never reaches the i8042 OR
  # the virtio-keyboard, and device-addressed input-send-event aborts
  # QEMU (object_property_find_err: 'qemu-fixed-text-console.device').
  $qemuArgs += @('-serial', "tcp:127.0.0.1:${serialPort},server,nowait")
} else {
  $qemuArgs += @('-serial', "file:$serial")
}
$qemuArgs += @('-display','none','-no-reboot','-no-shutdown')

# --- serial-over-TCP plumbing (typing mode) ---------------------------
$script:serialStream = $null
$script:serialFs = $null
$script:serialBuf = New-Object byte[] 65536

# Drain any bytes QEMU has emitted on the serial socket into the
# serial.log file, so the banner-wait/assert logic reads one source of
# truth regardless of transport.
function Pump-Serial {
  if (-not $script:serialStream) { return }
  while ($script:serialStream.DataAvailable) {
    $n = $script:serialStream.Read($script:serialBuf, 0, $script:serialBuf.Length)
    if ($n -le 0) { break }
    $script:serialFs.Write($script:serialBuf, 0, $n)
    $script:serialFs.Flush()
  }
}

function Connect-Serial([int]$port) {
  for ($i = 0; $i -lt 30; $i++) {
    try {
      $c = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port)
      $script:serialStream = $c.GetStream()
      $script:serialFs = [System.IO.File]::Open($serial, 'Append', 'Write', 'Read')
      return
    } catch { Start-Sleep -Milliseconds 500 }
  }
  throw "could not connect to QEMU serial TCP port $port"
}

function Send-SerialLine([string]$line) {
  $bytes = [System.Text.Encoding]::ASCII.GetBytes($line + "`r")
  $script:serialStream.Write($bytes, 0, $bytes.Length)
  $script:serialStream.Flush()
}

# Capture a PNG of the guest display via QMP screendump — the Unified
# REPL renders its scrollback on the virtio-gpu surface, so the screen
# is the only place a UI-side response is observable from a headless
# harness. (Typing itself goes over the serial console; QEMU's
# emulated-input injection is unreliable headless — see the -serial
# tcp note above.)
function Send-QmpScreendump([int]$port, [string]$screendumpTo) {
  $client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $port)
  try {
    $stream = $client.GetStream()
    $stream.ReadTimeout = 5000
    $reader = New-Object System.IO.StreamReader($stream)
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.AutoFlush = $true
    $null = $reader.ReadLine()                       # QMP greeting
    $writer.WriteLine('{"execute":"qmp_capabilities"}')
    $null = $reader.ReadLine()                       # {"return": {}}
    $png = $screendumpTo -replace '\\','/'
    $writer.WriteLine('{"execute":"screendump","arguments":{"filename":"' + $png + '","format":"png"}}')
    $null = $reader.ReadLine()
  } finally {
    $client.Close()
  }
}

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
if ($TypeLine) { Connect-Serial $serialPort }
$deadline = (Get-Date).AddSeconds($TimeoutSec)
$bannerSeen = $false
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Milliseconds 1000
  Pump-Serial
  if ($p.HasExited) { break }
  if (Test-Path $serial) {
    $txt = Get-Content $serial -Raw -ErrorAction SilentlyContinue
    if ($txt -and ($txt -match 'launcher running')) { $bannerSeen = $true; break }
  }
}
if ($TypeLine -and $bannerSeen -and -not $p.HasExited) {
  # Give the REPL drain loop a beat past the banner, then type.
  Start-Sleep -Milliseconds 1500
  Pump-Serial
  Write-Host "Typing into REPL via serial console: $TypeLine" -ForegroundColor Cyan
  Send-SerialLine $TypeLine
  # Follow-up lines (guest stdin, #476e): give the exec'd guest a beat
  # to reach its blocking read, then feed each line. Debug-profile
  # musl/ash startup under TCG is slow — 6 s settle is empirical.
  foreach ($line in $ThenType) {
    Start-Sleep -Milliseconds 6000
    Pump-Serial
    Write-Host "Typing into guest stdin: $line" -ForegroundColor Cyan
    Send-SerialLine $line
  }
  # Wait for the post-type phrases (or the deadline).
  $typeDeadline = (Get-Date).AddSeconds([Math]::Max(20, $TimeoutSec / 3))
  while ((Get-Date) -lt $typeDeadline) {
    Start-Sleep -Milliseconds 1000
    Pump-Serial
    if ($p.HasExited) { break }
    $txt = Get-Content $serial -Raw -ErrorAction SilentlyContinue
    if ($txt) {
      $allSeen = $true
      foreach ($phrase in $ExpectAfter) {
        if ($txt -notmatch [regex]::Escape($phrase)) { $allSeen = $false; break }
      }
      if ($allSeen -and $ExpectAfter.Count -gt 0) { break }
    }
  }
  # Final screen capture for the GPU-side story (best-effort).
  try { Send-QmpScreendump $qmpPort (Join-Path $wd 'screen.png') } catch {}
}
if ($TypeLine) { Pump-Serial }
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 500
if ($TypeLine) { Pump-Serial; if ($script:serialFs) { $script:serialFs.Close() } }

$log = if (Test-Path $serial) { (Get-Content $serial -Raw) -replace "`r","" } else { '' }

if (-not $Smoke) {
  Write-Host "`n=== serial.log ===" -ForegroundColor DarkGray
  Write-Host $log
  if (-not $Keep) { Write-Host "(staging dir: $wd)" -ForegroundColor DarkGray }
  return
}

# Smoke: assert every required banner phrase (+ the post-type set).
if ($TypeLine) { $required = @($required) + @($ExpectAfter) }
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
