# Diagnostic: boot the staged kernel, send one QMP send-key, print every
# QMP response verbatim (the smoke harness discards them), then query
# input devices via query-mice + HMP info qtree excerpts.
param(
  [string]$EfiPath = "C:\Users\lippe\Repos\arest\crates\arest-kernel\target\x86_64-unknown-uefi\release\arest-kernel.efi",
  [int]$BootWaitSec = 90
)
$ErrorActionPreference = 'Stop'
$qemu = 'C:\Program Files\qemu\qemu-system-x86_64.exe'
$wd = Join-Path $env:TEMP 'arest-qmp-probe'
Remove-Item -Recurse -Force $wd -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $wd 'esp\EFI\BOOT') | Out-Null
Copy-Item $EfiPath (Join-Path $wd 'esp\EFI\BOOT\BOOTX64.EFI') -Force
$share = 'C:\Program Files\qemu\share'
Copy-Item (Join-Path $share 'edk2-x86_64-code.fd') (Join-Path $wd 'code.fd') -Force
Copy-Item (Join-Path $share 'edk2-i386-vars.fd') (Join-Path $wd 'vars.fd') -Force
$serial = Join-Path $wd 'serial.log'
$disk = Join-Path $wd 'disk.img'
$fs = [System.IO.File]::Create($disk); $fs.SetLength(16MB); $fs.Close()

$qemuArgs = @(
  '-machine','q35','-cpu','max','-m','512',
  '-drive',"if=pflash,format=raw,unit=0,readonly=on,file=$wd\code.fd",
  '-drive',"if=pflash,format=raw,unit=1,file=$wd\vars.fd",
  '-drive',"file=fat:rw:$wd\esp,format=raw,if=ide",
  '-netdev','user,id=net0','-device','virtio-net-pci,netdev=net0,disable-legacy=on',
  '-drive',"file=$disk,format=raw,if=none,id=disk0",
  '-device','virtio-blk-pci,drive=disk0,disable-legacy=on',
  '-device','virtio-gpu-pci',
  '-device','virtio-tablet-pci',
  '-qmp','tcp:127.0.0.1:4445,server,nowait',
  '-serial',"file:$serial",'-display','none','-no-reboot','-no-shutdown'
)
$p = Start-Process -FilePath $qemu -ArgumentList $qemuArgs -PassThru -NoNewWindow
$deadline = (Get-Date).AddSeconds($BootWaitSec)
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Milliseconds 1000
  $txt = Get-Content $serial -Raw -ErrorAction SilentlyContinue
  if ($txt -and ($txt -match 'launcher running')) { break }
}
Start-Sleep -Seconds 2

$client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', 4445)
$stream = $client.GetStream()
$stream.ReadTimeout = 5000
$reader = New-Object System.IO.StreamReader($stream)
$writer = New-Object System.IO.StreamWriter($stream)
$writer.AutoFlush = $true

function Cmd([string]$json) {
  Write-Host ">>> $json"
  $writer.WriteLine($json)
  # Read lines until we see a return/error (skip async events).
  for ($i = 0; $i -lt 6; $i++) {
    try { $line = $reader.ReadLine() } catch { Write-Host "<<< (read timeout)"; return }
    Write-Host "<<< $line"
    if ($line -match '"return"' -or $line -match '"error"') { return }
  }
}

Write-Host "<<< $($reader.ReadLine())"   # greeting
Cmd '{"execute":"qmp_capabilities"}'
Cmd '{"execute":"query-mice"}'
Cmd '{"execute":"send-key","arguments":{"keys":[{"type":"qcode","data":"r"}]}}'
Cmd '{"execute":"send-key","arguments":{"keys":[{"type":"qcode","data":"ret"}]}}'
Cmd '{"execute":"human-monitor-command","arguments":{"command-line":"info ps2 "}}'
Cmd '{"execute":"human-monitor-command","arguments":{"command-line":"sendkey a"}}'
Cmd '{"execute":"human-monitor-command","arguments":{"command-line":"info irq"}}'
Start-Sleep -Seconds 8
$client.Close()
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500
Write-Host "=== serial diag tail ==="
(Get-Content $serial -Raw) -replace "`r","" | Select-String -Pattern 'diag|arest>' | Select-Object -Last 4
Remove-Item -Recurse -Force $wd -ErrorAction SilentlyContinue
