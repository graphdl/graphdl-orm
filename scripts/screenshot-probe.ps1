# scripts/screenshot-probe.ps1
#
# Headless see-and-drive probe (task kernel-see-drive-surface): boot the
# already-built arest-kernel .efi under host QEMU with networking, wait
# for the HTTP listener, then GET /screen and save the framebuffer PNG
# the kernel encodes. This is the agent-facing half of the loop that used
# to require a human watching a QEMU window -- the saved PNG is Read'able.
#
# Assumes the .efi already exists (build first, e.g. server,static-ip or
# mini,static-ip for x86_64-unknown-uefi). Socket-free host curl, no QEMU
# monitor -> doesn't trip the AMSI/ClickFix heuristic (#598).
#
# Usage:  pwsh scripts\screenshot-probe.ps1 [-Out target/screen-probe/screen.png]

param(
    [string]$Out = "target/screen-probe/screen.png"
)

$ErrorActionPreference = "Stop"
$repoRoot  = (Resolve-Path "$PSScriptRoot\..").Path
$kernelDir = Join-Path $repoRoot "crates\arest-kernel"
$efi       = Join-Path $kernelDir "target\x86_64-unknown-uefi\release\arest-kernel.efi"
if (-not (Test-Path $efi)) { throw "no .efi at $efi -- build the kernel for x86_64-unknown-uefi first" }

$stageRel  = "target/screen-probe"
$stageAbs  = Join-Path $repoRoot "target\screen-probe"
$espRel    = "$stageRel/esp"
$espAbs    = Join-Path $stageAbs "esp"
$serialAbs = Join-Path $stageAbs "serial.log"
$outAbs    = Join-Path $repoRoot $Out

$qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path $qemu)) {
    $c = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
    if ($c) { $qemu = $c.Source } else { throw "qemu-system-x86_64 not found" }
}
$share = Join-Path (Split-Path $qemu -Parent) "share"

# Stage ESP + OVMF (pflash split) + a scratch virtio-blk disk -- mirrors
# boot-kernel-uefi-mini.ps1's working recipe.
New-Item -ItemType Directory -Force -Path (Join-Path $espAbs "EFI\BOOT") | Out-Null
Copy-Item $efi (Join-Path $espAbs "EFI\BOOT\BOOTX64.EFI") -Force
Copy-Item (Join-Path $share "edk2-x86_64-code.fd") (Join-Path $stageAbs "ovmf-code.fd") -Force
Copy-Item (Join-Path $share "edk2-i386-vars.fd")  (Join-Path $stageAbs "ovmf-vars.fd")  -Force
$disk = Join-Path $stageAbs "disk.img"
if (-not (Test-Path $disk)) { fsutil file createnew $disk 10485760 | Out-Null }
if (Test-Path $serialAbs) { Clear-Content $serialAbs -ErrorAction SilentlyContinue }

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
    "-serial", "chardev:ser0",
    "-display", "none", "-no-reboot", "-no-shutdown"
)

Write-Host "Booting kernel headless for /screen probe..." -ForegroundColor Cyan
$p = Start-Process -FilePath $qemu -ArgumentList $qargs -PassThru -NoNewWindow -WorkingDirectory $repoRoot
$code = $null
try {
    # Wait for the HTTP listener / net-loop beacon (host TCG is slow).
    $deadline = (Get-Date).AddSeconds(150)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 750
        $log = (Get-Content $serialAbs -Raw -ErrorAction SilentlyContinue)
        if ($null -eq $log) { continue }
        if ($log -match "net\+http loop running" -or $log -match "handler registered on :80") { $ready = $true; break }
        if ($log -match "UEFI kernel panic") { Write-Host "PANIC in serial log." -ForegroundColor Red; break }
    }
    if (-not $ready) { Write-Host "WARN: HTTP beacon not seen in 150s; attempting curl anyway." -ForegroundColor Yellow }

    New-Item -ItemType Directory -Force -Path (Split-Path $outAbs -Parent) | Out-Null
    $curlOk = $false
    $deadlineC = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadlineC) {
        $code = (& curl.exe -s -m 5 -o $outAbs -w "%{http_code}" "http://127.0.0.1:8080/screen" 2>$null)
        if ($LASTEXITCODE -eq 0 -and $code -match '^[0-9]+$' -and [int]$code -ge 200 -and [int]$code -lt 300 -and (Test-Path $outAbs) -and (Get-Item $outAbs).Length -gt 0) { $curlOk = $true; break }
        Start-Sleep -Milliseconds 1000
    }
    if ($curlOk) {
        $sz = (Get-Item $outAbs).Length
        Write-Host "PASS: GET /screen -> HTTP $code, $sz bytes saved to $outAbs" -ForegroundColor Green
    } else {
        Write-Host "FAIL: GET /screen did not return a PNG (last code: $code)." -ForegroundColor Red
        Write-Host "--- serial tail ---"
        Get-Content $serialAbs -Tail 30 -ErrorAction SilentlyContinue
    }
} finally {
    if (-not $p.HasExited) { $p.Kill() }
}
