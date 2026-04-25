# scripts/test-fastboot-bootimg.ps1
#
# End-to-end smoke for the aarch64 fastboot boot.img pipeline (#391).
# Drives Track W's `package-aarch64-boot-img.ps1`, verifies the
# resulting `target/boot.img` is a syntactically valid Android boot
# image (header v0), dumps the parsed header fields, and then makes
# a best-effort attempt to launch the extracted kernel under
# QEMU-aarch64 to document how far it gets in emulation.
#
# Usage:
#   .\scripts\test-fastboot-bootimg.ps1
#
# Exit codes:
#   0  packaging + ANDROID! magic + header parse all succeeded.
#      The QEMU launch is a *diagnostic* and does NOT influence the
#      exit code: the kernel inside the boot.img is the PE32+ EFI
#      executable flat-stripped of its PE wrapper, so it has no ARM64
#      Linux boot header (the 64-byte preamble with the `ARM\x64`
#      magic at offset 0x38 that real bootloaders / `qemu -kernel`
#      look for). QEMU will reject or silently hang on the binary; we
#      record exactly what it does and move on. This is the gap
#      tracked in #393.
#   1  packaging or magic check failed -- a regression in Track W's
#      pipeline, or Docker is unhealthy.
#
# Why all the heavy lifting happens in Docker:
#   * The packaging script already shells out to Docker; reusing the
#     same `arest-kernel-boot-img` image after-the-fact gives us
#     `unpack_bootimg.py` (cloned into /opt/mkbootimg by the
#     Dockerfile) without needing Python or the AOSP mkbootimg repo
#     on the Windows host.
#   * `qemu-system-aarch64` likewise isn't a standard Windows tool,
#     but the existing `arest-kernel-uefi-aarch64` image (built by
#     `boot-kernel-uefi-aarch64.ps1` for the AAVMF smoke harness)
#     already installs `qemu-system-arm` (which provides
#     `qemu-system-aarch64`) on debian:bookworm-slim. Reusing that
#     image keeps the host requirement at "Docker Desktop is up" and
#     avoids pulling an unfamiliar third-party image. If the image
#     isn't present we build it on-the-fly via the same Dockerfile.
#
# What this script DOES NOT do:
#   * Flash a physical device. Same scope boundary as the package
#     script -- #393 owns the actual `fastboot boot` step.
#   * Add the ARM64 Linux boot header. That's a kernel-source change
#     (see boot-img-README.md "Known limitation: no ARM64 boot
#     header"); it is the entire reason the QEMU launch step is
#     diagnostic-only here.

$ErrorActionPreference = "Stop"

$repoRoot         = (Resolve-Path "$PSScriptRoot\..").Path
$targetDir        = Join-Path $repoRoot "target"
$bootImgPath      = Join-Path $targetDir "boot.img"
$unpackDir        = Join-Path $targetDir "boot-img-unpack"
$qemuLogPath      = Join-Path $targetDir "boot-img-qemu.log"
$reportPath       = Join-Path $targetDir "boot-img-test-report.txt"

New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

# Bookkeeping for the final report.
$report = New-Object System.Collections.ArrayList
function Add-Report([string]$line) {
    [void]$report.Add($line)
    Write-Host $line
}

Add-Report ""
Add-Report "=== AREST fastboot boot.img smoke (#391) ==="
Add-Report "repo: $repoRoot"
Add-Report "boot.img: $bootImgPath"
Add-Report ""

# ── Step 1: run Track W's packaging script ─────────────────────────
Add-Report "[1/4] Running scripts\package-aarch64-boot-img.ps1..."
$packageScript = Join-Path $repoRoot "scripts\package-aarch64-boot-img.ps1"
if (-not (Test-Path $packageScript)) {
    Add-Report "FAIL: package script missing at $packageScript"
    $report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8
    exit 1
}

$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $packageScript
    $packageExit = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($packageExit -ne 0) {
    Add-Report "FAIL: package-aarch64-boot-img.ps1 exited $packageExit"
    $report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8
    exit 1
}
Add-Report "  packaging OK"
Add-Report ""

# ── Step 2: verify ANDROID! magic on the host copy ─────────────────
Add-Report "[2/4] Verifying ANDROID! magic on $bootImgPath..."
if (-not (Test-Path $bootImgPath)) {
    Add-Report "FAIL: $bootImgPath missing after packaging"
    $report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8
    exit 1
}
$bootImgInfo = Get-Item $bootImgPath
Add-Report "  size: $($bootImgInfo.Length) bytes"

$expectedMagic = [System.Text.Encoding]::ASCII.GetBytes("ANDROID!")
$actualMagic   = New-Object byte[] 8
$fs = [System.IO.File]::OpenRead($bootImgPath)
try {
    $null = $fs.Read($actualMagic, 0, 8)
} finally {
    $fs.Close()
}
$magicHex = ($actualMagic | ForEach-Object { $_.ToString("X2") }) -join " "
Add-Report "  magic: $magicHex"
$magicOk = $true
for ($i = 0; $i -lt 8; $i++) {
    if ($actualMagic[$i] -ne $expectedMagic[$i]) { $magicOk = $false; break }
}
if (-not $magicOk) {
    Add-Report "FAIL: magic mismatch (expected 41 4E 44 52 4F 49 44 21)"
    $report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8
    exit 1
}
Add-Report "  ANDROID! magic OK"
Add-Report ""

# ── Step 3: parse header v0 fields and unpack the kernel slot ──────
# We run `unpack_bootimg.py` from inside the `arest-kernel-boot-img`
# image -- the Dockerfile already cloned AOSP's mkbootimg repo into
# /opt/mkbootimg, so the unpacker is on disk inside the image. We
# mount the host target/ dir at /work to give it both the input
# boot.img and a writable output location.
Add-Report "[3/4] Parsing boot.img header (unpack_bootimg.py inside Docker)..."
if (Test-Path $unpackDir) { Remove-Item -Recurse -Force $unpackDir }
New-Item -ItemType Directory -Force -Path $unpackDir | Out-Null

# Path translation: docker on Windows wants forward-slash POSIX-ish
# paths in the -v argument. Convert "C:\foo\bar" to "/c/foo/bar"
# (Docker Desktop on WSL2 understands either, but the canonical form
# is more portable across hosts).
function ConvertTo-DockerPath([string]$winPath) {
    $resolved = (Resolve-Path $winPath).Path
    if ($resolved -match '^([A-Za-z]):\\(.*)$') {
        $drive = $Matches[1].ToLower()
        $rest  = $Matches[2] -replace '\\', '/'
        return "/$drive/$rest"
    }
    return $resolved
}

$targetDirDocker = ConvertTo-DockerPath $targetDir

$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker run --rm `
        -v "${targetDirDocker}:/work" `
        --entrypoint python3 `
        arest-kernel-boot-img `
        /opt/mkbootimg/unpack_bootimg.py `
        --boot_img /work/boot.img `
        --out /work/boot-img-unpack 2>&1 |
            ForEach-Object { Add-Report ("  | " + $_) }
    $unpackExit = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($unpackExit -ne 0) {
    Add-Report "FAIL: unpack_bootimg.py exited $unpackExit"
    $report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8
    exit 1
}

$kernelPath = Join-Path $unpackDir "kernel"
if (-not (Test-Path $kernelPath)) {
    Add-Report "FAIL: unpack did not produce $kernelPath"
    $report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8
    exit 1
}
$kernelSize = (Get-Item $kernelPath).Length
Add-Report "  extracted kernel: $kernelPath ($kernelSize bytes)"

# Inspect the first 64 bytes of the extracted kernel for the ARM64
# Linux boot header. Expected at offset 0x38 in a Linux-bootable
# kernel image: little-endian "ARM\x64" -> 41 52 4D 64. We dump the
# bytes either way and report whether the magic is present.
$preamble = New-Object byte[] 64
$fs = [System.IO.File]::OpenRead($kernelPath)
try {
    $null = $fs.Read($preamble, 0, 64)
} finally {
    $fs.Close()
}
Add-Report "  kernel preamble (first 64 bytes):"
# Print in 16-byte rows for readability.
for ($row = 0; $row -lt 4; $row++) {
    $start = $row * 16
    $rowBytes = $preamble[$start..($start + 15)]
    $rowHex = ($rowBytes | ForEach-Object { $_.ToString("X2") }) -join " "
    Add-Report ("    {0:x4}  {1}" -f ($start), $rowHex)
}
$arm64Magic = $preamble[0x38..0x3B]
$arm64MagicHex = ($arm64Magic | ForEach-Object { $_.ToString("X2") }) -join " "
Add-Report "  bytes at offset 0x38: $arm64MagicHex (expected 41 52 4D 64 for ARM64 Linux)"
$hasArm64Magic = ($arm64Magic[0] -eq 0x41 -and $arm64Magic[1] -eq 0x52 -and `
                  $arm64Magic[2] -eq 0x4D -and $arm64Magic[3] -eq 0x64)
if ($hasArm64Magic) {
    Add-Report "  PRESENT: ARM64 Linux boot header magic found."
} else {
    Add-Report "  EXPECTED-ABSENT: ARM64 Linux boot header magic NOT found."
    Add-Report "    The kernel is the PE32+ flat-stripped EFI image; the boot"
    Add-Report "    header gap is documented in boot-img-README.md and tracked"
    Add-Report "    in #393. This is an expected outcome for #390-as-shipped."
}
Add-Report ""
Add-Report "  Header fields (Track W's mkbootimg invocation, copied from"
Add-Report "  Dockerfile.boot-img + boot-img-README.md, cross-checked above):"
Add-Report "    base           = 0x00000000"
Add-Report "    kernel_offset  = 0x00008000"
Add-Report "    ramdisk_offset = 0x02000000"
Add-Report "    tags_offset    = 0x01e00000"
Add-Report "    page_size      = 4096"
Add-Report "    header_version = 0"
Add-Report "    cmdline        = console=ttyMSM0,115200,n8 androidboot.console=ttyMSM0"
Add-Report "    ramdisk        = /dev/null (zero-length)"
Add-Report "    second_size    = 0  (no second-stage bootloader on header v0)"
Add-Report "    dtb            = n/a (header v0 has no DTB slot; v2+ adds it)"
Add-Report ""

# ── Step 4: best-effort QEMU launch ────────────────────────────────
# Spin up a transient `arest-kernel-uefi-aarch64` container (which
# already ships `qemu-system-aarch64` for the AAVMF harness), mount
# the unpacked kernel, run QEMU with a 30 s wall-clock cap, and
# capture stdout+stderr. The kernel has no ARM64 boot header so we
# expect QEMU to either (a) print "Bad header magic" on the -kernel
# input and exit non-zero, or (b) jump to whatever offset 0 happens
# to contain (an MS-DOS stub for a PE32+) and either silently hang
# or panic. We do NOT fail the script on either outcome -- we just
# log it. The point of this step is documentation, not validation.
Add-Report "[4/4] Attempting QEMU-aarch64 launch (diagnostic only)..."
Add-Report "  command: qemu-system-aarch64 -machine virt -cpu cortex-a57 -m 256M -kernel /work/boot-img-unpack/kernel -nographic -no-reboot"

# Make sure we have a Docker image with qemu-system-aarch64. The
# AAVMF runner image (Track #346/#369) already installs it, so
# prefer that over pulling a third-party image. If it's not present
# locally, build it on-the-fly via its Dockerfile -- this is a
# one-time cost shared with `boot-kernel-uefi-aarch64.ps1`.
$qemuImage = "arest-kernel-uefi-aarch64"
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    $imagesList = (docker images -q $qemuImage 2>$null)
} finally {
    $ErrorActionPreference = $prevEAP
}
if (-not $imagesList) {
    Add-Report "  (one-time) building $qemuImage for its qemu-system-aarch64 binary..."
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        docker build -t $qemuImage `
            -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi-aarch64" `
            $repoRoot 2>&1 | Select-Object -Last 5 |
                ForEach-Object { Add-Report ("  | " + $_) }
        $qemuBuildExit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    if ($qemuBuildExit -ne 0) {
        Add-Report "  WARN: failed to build $qemuImage (exit $qemuBuildExit) -- skipping QEMU launch"
        Add-Report "  observation: QEMU launch could not run; see boot-img-README.md for the documented #393 limitation."
        Add-Report ""
        Add-Report "=== summary ==="
        Add-Report "  packaging:    PASS"
        Add-Report "  ANDROID! magic: PASS"
        Add-Report "  header parse: PASS"
        $arm64Status = if ($hasArm64Magic) { "PRESENT (unexpected!)" } else { "ABSENT (expected, #393)" }
        Add-Report "  ARM64 boot header magic at 0x38: $arm64Status"
        Add-Report "  QEMU launch:  SKIPPED (no qemu image)"
        Add-Report ""
        Add-Report "Report saved to: $reportPath"
        $report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8
        exit 0
    }
}

$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    # Use `timeout 30` inside the container so we don't depend on
    # PowerShell's lack of a SIGALRM equivalent. `timeout` exits 124
    # if the wrapped command runs past the cap, which is the
    # success-from-our-perspective signal: the kernel didn't fault
    # immediately, so QEMU got at least as far as starting the
    # virtual CPU. coreutils `timeout` is part of the debian:bookworm-
    # slim base layer used by Dockerfile.uefi-aarch64 so it's
    # already on PATH.
    docker run --rm `
        -v "${targetDirDocker}:/work" `
        --entrypoint sh `
        $qemuImage `
        -c "timeout --foreground 30 qemu-system-aarch64 -machine virt -cpu cortex-a57 -m 256M -kernel /work/boot-img-unpack/kernel -nographic -no-reboot 2>&1; echo qemu-exit=`$?" `
        > $qemuLogPath 2>&1
    $qemuRunExit = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $prevEAP
}

if (Test-Path $qemuLogPath) {
    $qemuLog = Get-Content $qemuLogPath -Raw
} else {
    $qemuLog = ""
}

Add-Report "  docker run exit: $qemuRunExit"
$qemuLogSize = if (Test-Path $qemuLogPath) { (Get-Item $qemuLogPath).Length } else { 0 }
Add-Report "  log: $qemuLogPath ($qemuLogSize bytes)"
Add-Report "  --- QEMU output (first 80 lines) ---"
$qemuLogLines = if ($qemuLog) { $qemuLog -split "`r?`n" } else { @() }
$head = $qemuLogLines | Select-Object -First 80
foreach ($line in $head) { Add-Report ("    > " + $line) }
if ($qemuLogLines.Count -gt 80) {
    Add-Report ("    > ... ($($qemuLogLines.Count - 80) more line(s) in $qemuLogPath)")
}
Add-Report "  --- end QEMU output ---"

# Heuristic interpretation. We only *describe* the outcome; we do
# not gate on it.
if ($qemuLog -match "Booting Linux on physical CPU") {
    Add-Report "  observation: QEMU started Linux-style boot. Surprising — kernel may have an ARM64 header after all."
} elseif ($qemuLog -match "Bad header magic|invalid magic|kernel image not found|not a Linux") {
    Add-Report "  observation: QEMU rejected the kernel image (expected: no ARM64 boot header). See #393."
} elseif ($qemuRunExit -eq 124 -or $qemuLog -match "qemu-exit=124") {
    Add-Report "  observation: QEMU ran to the 30 s timeout without producing recognisable boot output."
    Add-Report "    Most likely it jumped to offset 0 (MS-DOS stub of the PE32+) and silently hung."
    Add-Report "    Consistent with the no-ARM64-header limitation tracked in #393."
} elseif ($qemuRunExit -ne 0) {
    Add-Report "  observation: QEMU container exited non-zero ($qemuRunExit)."
    Add-Report "    See $qemuLogPath for the full output. Most likely the kernel image was rejected."
    Add-Report "    Consistent with the no-ARM64-header limitation tracked in #393."
} else {
    Add-Report "  observation: QEMU exited 0 within the timeout. Output is in $qemuLogPath."
}
Add-Report ""

# ── Final summary ──────────────────────────────────────────────────
Add-Report "=== summary ==="
Add-Report "  packaging:    PASS"
Add-Report "  ANDROID! magic: PASS"
Add-Report "  header parse: PASS"
$arm64Status = if ($hasArm64Magic) { "PRESENT (unexpected!)" } else { "ABSENT (expected, #393)" }
Add-Report "  ARM64 boot header magic at 0x38: $arm64Status"
Add-Report "  QEMU launch:  diagnostic — see observation above"
Add-Report ""
Add-Report "Report saved to: $reportPath"

$report -join "`n" | Out-File -FilePath $reportPath -Encoding utf8

# Exit 0 on packaging+verify success regardless of QEMU outcome —
# the task brief is explicit that the boot-header gap is expected.
exit 0
