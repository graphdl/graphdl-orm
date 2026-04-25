# scripts/package-aarch64-boot-img.ps1
#
# Wraps the aarch64-unknown-uefi kernel into an Android boot.img
# (header v0) for fastboot-flash on Nexus 5X (msm8992 / bullhead).
# Drives the multi-stage `Dockerfile.boot-img` and extracts the
# resulting `boot.img` to `target/boot.img` on the host.
#
# Usage:
#   .\scripts\package-aarch64-boot-img.ps1
#
# Output:
#   target/boot.img -- Android boot image (magic bytes "ANDROID!")
#
# What this script DOESN'T do:
#   * Flash a physical device. The Nexus 5X flashing flow lives in
#     #393 and needs `fastboot` on the host plus a device in
#     bootloader mode. This script is the packaging-only step.
#   * Boot the artifact under emulation. Android's bootloader
#     parses the boot.img header and relocates the kernel to a
#     device-specific physical address before jumping in -- there's
#     no emulator that mimics the Snapdragon 808 boot flow closely
#     enough to be useful. The smoke for this artifact is the
#     ANDROID! magic check, not an actual boot.
#
# Known limitation -- carried forward from boot-img-README.md:
#   The kernel slot inside the boot.img is the PE32+ .efi flattened
#   to a raw binary. It does NOT carry an ARM64 boot protocol
#   header (the 8-byte branch + "ARM\x64" magic at offset 0x38
#   that Linux kernels and Android-friendly bare-metal kernels put
#   at offset 0). The Nexus 5X bootloader will reject it with a
#   "missing arm64 magic" error the moment a real device tries to
#   load it. Adding the header to the kernel itself is a separate
#   task. This script's success criterion stops at "boot.img
#   produced + ANDROID! magic present".

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path
$targetDir = Join-Path $repoRoot "target"
$bootImgHostPath = Join-Path $targetDir "boot.img"

New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

# docker writes BuildKit progress to stderr; PowerShell 5.1 wraps
# each stderr line in an ErrorRecord (NativeCommandError) when
# streams are merged, which fires under $ErrorActionPreference =
# "Stop". Mirror the existing UEFI scripts: relax error-action
# around native-exec calls and gate on $LASTEXITCODE.
Write-Host "Building Android boot.img wrapper image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-boot-img `
        -f "$repoRoot\crates\arest-kernel\Dockerfile.boot-img" `
        $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

# Extract boot.img from the built image. `docker create` makes a
# stopped container we can `docker cp` from without ever running
# it -- avoids the noise of CMD's banner while still reaching the
# image's filesystem.
$containerName = "arest-kernel-boot-img-extract-$([guid]::NewGuid().ToString('N').Substring(0,8))"

Write-Host "`nExtracting boot.img to $bootImgHostPath..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker create --name $containerName arest-kernel-boot-img | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "docker create failed (exit $LASTEXITCODE)" }

    try {
        docker cp "${containerName}:/boot.img" $bootImgHostPath
        if ($LASTEXITCODE -ne 0) { throw "docker cp failed (exit $LASTEXITCODE)" }
    }
    finally {
        docker rm $containerName 2>&1 | Out-Null
    }
} finally {
    $ErrorActionPreference = $prevEAP
}

# Verify the artifact: non-zero size + "ANDROID!" magic at offset 0.
# This is the same gate the Dockerfile's RUN check runs in-image,
# repeated here so a host-side regression (corrupt copy, wrong
# extraction path) is caught before the user tries to flash.
if (-not (Test-Path $bootImgHostPath)) {
    throw "boot.img not found at $bootImgHostPath after extraction"
}

$bootImgInfo = Get-Item $bootImgHostPath
if ($bootImgInfo.Length -eq 0) {
    throw "boot.img at $bootImgHostPath is zero bytes"
}

# Read the first 8 bytes and compare to "ANDROID!". Use a byte
# array (not a string) so PowerShell 5.1's UTF-16 default encoding
# doesn't mangle the comparison.
$expectedMagic = [System.Text.Encoding]::ASCII.GetBytes("ANDROID!")
$actualMagic = New-Object byte[] 8
$fs = [System.IO.File]::OpenRead($bootImgHostPath)
try {
    $null = $fs.Read($actualMagic, 0, 8)
} finally {
    $fs.Close()
}

$magicOk = $true
for ($i = 0; $i -lt 8; $i++) {
    if ($actualMagic[$i] -ne $expectedMagic[$i]) { $magicOk = $false; break }
}

if (-not $magicOk) {
    $hex = ($actualMagic | ForEach-Object { $_.ToString("X2") }) -join " "
    throw "boot.img magic check failed: expected 'ANDROID!' (41 4E 44 52 4F 49 44 21), got $hex"
}

Write-Host ""
Write-Host "PASS: boot.img produced and verified." -ForegroundColor Green
Write-Host "  Path:  $bootImgHostPath"
Write-Host "  Size:  $($bootImgInfo.Length) bytes"
Write-Host "  Magic: ANDROID! (header v0, Nexus 5X bullhead offsets)"
Write-Host ""
Write-Host "Next step (#393): fastboot boot $bootImgHostPath" -ForegroundColor DarkGray
Write-Host "Note: the kernel slot inside the boot.img has no ARM64 boot header" -ForegroundColor DarkGray
Write-Host "      (the PE32+ flatten step drops the entry-point metadata) so" -ForegroundColor DarkGray
Write-Host "      the bootloader will reject it until that header is added." -ForegroundColor DarkGray
Write-Host "      See crates/arest-kernel/boot-img-README.md." -ForegroundColor DarkGray
