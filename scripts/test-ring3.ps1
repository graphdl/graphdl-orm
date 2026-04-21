# scripts/test-ring3.ps1
#
# Ring-3 smoke test harness. Builds arest-kernel with --features
# ring3-smoke, produces a BIOS image via arest-kernel-image, runs
# the image under qemu-system-x86_64 with isa-debug-exit plumbing,
# and returns 0 on QEMU exit code 33 (kernel exit code 0x10 = smoke
# passed). On any other result, the QEMU serial log is printed for
# diagnosis.

$ErrorActionPreference = "Stop"

# Repo root = this script's parent directory.
$RepoRoot   = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$Target     = Join-Path $RepoRoot "target"
$KernelDir  = Join-Path $RepoRoot "crates\arest-kernel"
$ImageDir   = Join-Path $RepoRoot "crates\arest-kernel-image"
$KernelElf  = Join-Path $KernelDir "target\x86_64-unknown-none\debug\arest-kernel"
$ImagePath  = Join-Path $Target "arest-kernel-ring3.img"
$SerialLog  = Join-Path $Target "test-serial.log"

# Ensure target/ exists for the serial log + image output.
New-Item -ItemType Directory -Force -Path $Target | Out-Null

# Locate QEMU — prefer PATH, fall back to the standard Windows install
# location. Using the call operator (&) lets us invoke through a
# full path with spaces without relying on the user's PATH.
$Qemu = $null
$fromPath = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
if ($fromPath) {
    $Qemu = $fromPath.Path
} elseif (Test-Path "C:\Program Files\qemu\qemu-system-x86_64.exe") {
    $Qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
} else {
    Write-Error "qemu-system-x86_64 not found on PATH or at C:\Program Files\qemu\"
    exit 5
}

# 1. Build the kernel with the smoke-test feature enabled.
Push-Location $KernelDir
try {
    cargo build --features ring3-smoke --target x86_64-unknown-none
    if ($LASTEXITCODE -ne 0) { Write-Error "kernel build failed"; exit 10 }
} finally {
    Pop-Location
}

if (-not (Test-Path $KernelElf)) {
    Write-Error "kernel ELF missing at $KernelElf"
    exit 11
}

# 2. Produce the BIOS image.
Push-Location $ImageDir
try {
    cargo run -- $KernelElf $ImagePath
    if ($LASTEXITCODE -ne 0) { Write-Error "image build failed"; exit 12 }
} finally {
    Pop-Location
}

# 3. Launch QEMU. isa-debug-exit lets the kernel exit QEMU with a
#    specific code by writing a u32 to port 0xf4.
$QemuArgs = @(
    "-drive",  "format=raw,file=$ImagePath",
    "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-serial", "file:$SerialLog",
    "-display","none",
    "-no-reboot","-no-shutdown"
)

# Start QEMU via the call operator so it inherits stdio properly and
# its return code comes back via $LASTEXITCODE. We enforce the timeout
# by running in a System.Diagnostics.Process and polling WaitForExit.
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Qemu
foreach ($arg in $QemuArgs) { [void]$psi.ArgumentList.Add($arg) }
$psi.UseShellExecute = $false
$psi.RedirectStandardOutput = $false
$psi.RedirectStandardError = $false
$psi.CreateNoWindow = $true
$qemu = [System.Diagnostics.Process]::Start($psi)
if (-not $qemu.WaitForExit(30000)) {
    $qemu.Kill()
    Write-Host "-- serial log (timeout) --"
    if (Test-Path $SerialLog) { Get-Content $SerialLog }
    exit 2
}

# 4. Translate QEMU exit code to harness exit code.
#    Kernel 0x10  -> QEMU (0x10<<1)|1 = 33   -> success
#    Kernel 0x11  -> QEMU (0x11<<1)|1 = 35   -> ring-3 fault
#    Kernel 0xFF  -> QEMU (0xFF<<1)|1 = 511  -> kernel panic
switch ($qemu.ExitCode) {
    33 {
        exit 0
    }
    35 {
        Write-Host "-- serial log (ring-3 fault exit 0x11) --"
        if (Test-Path $SerialLog) { Get-Content $SerialLog }
        exit 3
    }
    511 {
        Write-Host "-- serial log (kernel panic exit 0xFF) --"
        if (Test-Path $SerialLog) { Get-Content $SerialLog }
        exit 4
    }
    default {
        Write-Host "-- serial log (unexpected QEMU exit $($qemu.ExitCode)) --"
        if (Test-Path $SerialLog) { Get-Content $SerialLog }
        exit 1
    }
}
