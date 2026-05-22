#!/usr/bin/env pwsh
# Run a command (typically a cargo/wasm build or test) with its ENTIRE
# process tree capped to a committed-memory ceiling via a Windows Job Object.
#
# WHY: a runaway compile (rustc/LLVM) on a memory-constrained box can exhaust
# all of system RAM and take the OS down. A build should never be able to do
# that. With a Job Object memory cap, an allocation past the ceiling FAILS
# inside the offending process — rustc/LLVM then abort with their normal
# "out of memory" error (a graceful build failure, exit != 0) — while the OS
# and other apps keep the headroom *below* the cap. The failure mode becomes
# "this build needs a lighter profile" instead of "the box rebooted".
#
# USAGE:
#   pwsh scripts/mem-capped-build.ps1 [-CapGB N | -CapPercent P] [-WorkDir path] -- <command...>
#
#   # default cap = 65% of total physical RAM, run from current dir:
#   pwsh scripts/mem-capped-build.ps1 -- cargo test --lib
#
#   # explicit 10 GB cap, in the engine crate, opt-0 profile via env:
#   $env:CARGO_PROFILE_TEST_OPT_LEVEL = '0'; $env:RUST_MIN_STACK = '67108864'
#   pwsh scripts/mem-capped-build.ps1 -CapGB 10 -WorkDir crates/arest -- cargo test --lib -j 2
#
# NOTES:
#   * Environment variables set before invoking are inherited by the child
#     (UseShellExecute=false), so CARGO_PROFILE_*/RUST_MIN_STACK pass through.
#   * KILL_ON_JOB_CLOSE: if this script is interrupted, the job's processes are
#     killed too — no orphaned rustc left holding memory.
#   * Arg values containing spaces are not quoted (build commands don't need
#     it); pass such a command differently if ever required.

[CmdletBinding(PositionalBinding = $false)]
param(
  [int]$CapGB = 0,
  [int]$CapPercent = 65,
  [string]$WorkDir = '',
  [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
  [string[]]$CommandArgs
)

if ($CommandArgs.Count -gt 0 -and $CommandArgs[0] -eq '--') {
  $CommandArgs = @($CommandArgs[1..($CommandArgs.Count - 1)])
}
if ($CommandArgs.Count -eq 0) { Write-Error 'No command given (expected: ... -- <command...>).'; exit 2 }

$total = [uint64](Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
$cap = if ($CapGB -gt 0) { [uint64]$CapGB * 1GB } else { [uint64][math]::Floor($total * $CapPercent / 100.0) }
$cwd = if ($WorkDir) { (Resolve-Path $WorkDir).Path } else { (Get-Location).Path }
Write-Host ("[mem-cap] total={0:N1}GB cap={1:N1}GB cwd={2}" -f ($total / 1GB), ($cap / 1GB), $cwd)
Write-Host ("[mem-cap] cmd: {0}" -f ($CommandArgs -join ' '))

Add-Type -TypeDefinition @"
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;

public static class MemCappedJob {
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
  static extern IntPtr CreateJobObjectW(IntPtr a, string name);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION info, uint len);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool AssignProcessToJobObject(IntPtr job, IntPtr proc);

  [StructLayout(LayoutKind.Sequential)]
  struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    public long PerProcessUserTimeLimit;
    public long PerJobUserTimeLimit;
    public uint LimitFlags;
    public UIntPtr MinimumWorkingSetSize;
    public UIntPtr MaximumWorkingSetSize;
    public uint ActiveProcessLimit;
    public UIntPtr Affinity;
    public uint PriorityClass;
    public uint SchedulingClass;
  }
  [StructLayout(LayoutKind.Sequential)]
  struct IO_COUNTERS { public ulong a, b, c, d, e, f; }
  [StructLayout(LayoutKind.Sequential)]
  struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
    public IO_COUNTERS IoInfo;
    public UIntPtr ProcessMemoryLimit;
    public UIntPtr JobMemoryLimit;
    public UIntPtr PeakProcessMemoryUsed;
    public UIntPtr PeakJobMemoryUsed;
  }

  const int JobObjectExtendedLimitInformation = 9;
  const uint JOB_OBJECT_LIMIT_JOB_MEMORY = 0x200;
  const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000;

  public static int Run(string fileName, string arguments, ulong cap, string workDir) {
    IntPtr job = CreateJobObjectW(IntPtr.Zero, null);
    if (job == IntPtr.Zero) throw new Exception("CreateJobObject failed: " + Marshal.GetLastWin32Error());

    var info = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_JOB_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    info.JobMemoryLimit = (UIntPtr)cap;
    if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, ref info, (uint)Marshal.SizeOf(info)))
      throw new Exception("SetInformationJobObject failed: " + Marshal.GetLastWin32Error());

    var psi = new ProcessStartInfo(fileName, arguments) { UseShellExecute = false };
    if (!string.IsNullOrEmpty(workDir)) psi.WorkingDirectory = workDir;
    var p = Process.Start(psi);
    // Assign immediately; child rustc procs spawned afterwards inherit job
    // membership. (cargo does dependency resolution before spawning rustc, so
    // the sub-millisecond window before assignment carries negligible risk.)
    if (!AssignProcessToJobObject(job, p.Handle))
      Console.Error.WriteLine("[mem-cap] WARN: AssignProcessToJobObject failed: " + Marshal.GetLastWin32Error());
    p.WaitForExit();
    return p.ExitCode;
  }
}
"@

$exe = $CommandArgs[0]
$resolved = (Get-Command $exe -ErrorAction SilentlyContinue).Source
if ($resolved) { $exe = $resolved }
$argLine = if ($CommandArgs.Count -gt 1) { ($CommandArgs[1..($CommandArgs.Count - 1)]) -join ' ' } else { '' }

$code = [MemCappedJob]::Run($exe, $argLine, [uint64]$cap, $cwd)
Write-Host ("[mem-cap] exit={0}" -f $code)
exit $code
