#!/usr/bin/env pwsh
# scripts/check-all-targets.ps1
#
# Cross-target compile sweep for AREST. Runs `cargo +nightly check
# --tests` (or just `--target` for kernel cross-compiles that lack a
# host test runner) against every supported target with the standard
# feature combinations, and reports per-target pass/fail in one
# summary block. Tracks #668.
#
# Why this exists: the silent-aarch64-break failure mode that bit
# #441/#452/#654 - a change lands that compiles cleanly on the host
# triple (x86_64-pc-windows-msvc) and the default UEFI target
# (x86_64-unknown-uefi), but quietly breaks one of the lean kernel
# server profiles (aarch64-unknown-uefi --features server,static-ip
# was the recurring offender). `cargo test --lib` doesn't catch it
# because the broken target isn't in the test runner's path. This
# script catches it by sweeping every supported target up front.
#
# Targets (single matrix - server profile only, the lean baseline):
#   1. x86_64-pc-windows-msvc (host)        - cargo check --tests -p arest -p arest-kernel
#   2. x86_64-unknown-uefi server,static-ip - kernel server profile
#   3. x86_64-unknown-uefi default          - kernel mini/full
#   4. aarch64-unknown-uefi server,static-ip
#   5. arm-unknown-uefi server,static-ip    - armv7 custom target
#   6. wasm32-unknown-unknown cloudflare    - arest worker build
#
# Per-target steps:
#   - rustup target add <target> --toolchain nightly  (try/catch -
#     already-installed isn't fatal; targets with no built-in spec
#     are SKIP rather than FAIL since they need -Z build-std)
#   - cargo +nightly check --target <target> [features] - must exit 0
#   - Capture warnings count from stderr ("warning:" lines)
#
# Exit code:
#   * 0 if every target either PASSED or was SKIPPED with a documented reason.
#   * 1 if any target FAILED.
#
# Usage:
#   .\scripts\check-all-targets.ps1            # default: only result line per target
#   .\scripts\check-all-targets.ps1 -Verbose   # also dump full stdout per target
#
# NOT wired into test-all.ps1 - #671 will plumb that in CI separately.

param(
    [switch]$VerboseOutput
)

$ErrorActionPreference = "Continue"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# ---------------------------------------------------------------------------
# Per-target result tracking
# ---------------------------------------------------------------------------
# Each target records: key, name (for display), status (PASS/FAIL/SKIP),
# warnings count, errors count, elapsed seconds, and full captured output
# (for -VerboseOutput).
$results = New-Object System.Collections.Generic.List[PSObject]

function Add-TargetResult {
    param(
        [string]$Key,
        [string]$Name,
        [string]$Status,    # PASS | FAIL | SKIP
        [int]$Warnings,
        [int]$Errors,
        [double]$Elapsed,
        [string]$Reason,
        [string]$Output
    )
    $results.Add([PSCustomObject]@{
        Key      = $Key
        Name     = $Name
        Status   = $Status
        Warnings = $Warnings
        Errors   = $Errors
        Elapsed  = $Elapsed
        Reason   = $Reason
        Output   = $Output
    })
}

# Run a native command, capture stdout+stderr asynchronously, return
# (exitCode, capturedOutput, elapsedSeconds). Lifted verbatim from
# scripts/test-all.ps1 - the PSI + BeginOutputReadLine pattern is the
# canonical Windows-correct path for capturing native cargo output
# without deadlocking. See test-all.ps1's Invoke-Capture for the long-
# form rationale; the short version: Start-Process -Wait deadlocks on
# long cargo runs, cmd.exe redirection mangles stderr through PS 5.1
# pipelines, this approach drains both pipes asynchronously and is
# bulletproof.
function Invoke-Capture {
    param(
        [string]$Cmd,
        [string[]]$CmdArgs,
        [string]$WorkingDir,
        [int]$TimeoutSec = 0
    )
    $start = Get-Date
    $exit = 0
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Cmd
    # PS 5.1's PSI takes a single Arguments string (no ArgumentList collection).
    # Quote args containing whitespace; pass others as-is.
    $argString = ($CmdArgs | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + ($_ -replace '"','\"') + '"' } else { $_ }
    }) -join ' '
    $psi.Arguments = $argString
    $psi.WorkingDirectory = $WorkingDir
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi
    $stdoutSb = New-Object System.Text.StringBuilder
    $stderrSb = New-Object System.Text.StringBuilder
    $stdoutEvent = Register-ObjectEvent -InputObject $proc `
        -EventName "OutputDataReceived" `
        -MessageData $stdoutSb `
        -Action { if ($null -ne $EventArgs.Data) { [void]$Event.MessageData.AppendLine($EventArgs.Data) } }
    $stderrEvent = Register-ObjectEvent -InputObject $proc `
        -EventName "ErrorDataReceived" `
        -MessageData $stderrSb `
        -Action { if ($null -ne $EventArgs.Data) { [void]$Event.MessageData.AppendLine($EventArgs.Data) } }
    try {
        [void]$proc.Start()
        $proc.BeginOutputReadLine()
        $proc.BeginErrorReadLine()
        if ($TimeoutSec -gt 0) {
            $exited = $proc.WaitForExit($TimeoutSec * 1000)
            if (-not $exited) {
                try {
                    Get-CimInstance Win32_Process -Filter "ParentProcessId=$($proc.Id)" -ErrorAction SilentlyContinue |
                        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
                    $proc.Kill()
                } catch {}
                [void]$stderrSb.AppendLine("Invoke-Capture: TIMEOUT after $TimeoutSec sec - killed process tree.")
                $exit = -1
            } else {
                $proc.WaitForExit()
                $exit = $proc.ExitCode
            }
        } else {
            $proc.WaitForExit()
            $exit = $proc.ExitCode
        }
    } catch {
        $exit = -1
        [void]$stderrSb.AppendLine("Process.Start threw: " + $_.Exception.Message)
    } finally {
        Unregister-Event -SourceIdentifier $stdoutEvent.Name -ErrorAction SilentlyContinue
        Unregister-Event -SourceIdentifier $stderrEvent.Name -ErrorAction SilentlyContinue
        $proc.Dispose()
    }
    $elapsed = ((Get-Date) - $start).TotalSeconds
    $outText = $stdoutSb.ToString()
    $errText = $stderrSb.ToString()
    if ($errText) { $outText += "`n--- stderr ---`n" + $errText }
    if ($null -eq $outText) { $outText = "" }
    return @{ Exit = $exit; Output = $outText; Elapsed = $elapsed }
}

# Count cargo warnings + errors from combined output. Cargo emits both
# prefixed at column 0; we count whole-line matches so an inner phrase
# like "warning: from previous run" doesn't double-tally.
function Count-Diagnostics {
    param([string]$Output)
    $warnings = ([regex]::Matches($Output, "(?m)^warning:")).Count
    $errors   = ([regex]::Matches($Output, "(?m)^error(\[E\d+\])?:")).Count
    return @{ Warnings = $warnings; Errors = $errors }
}

# Idempotently install a rustup target on the nightly toolchain. Returns
# $true if the target is available afterwards; $false if rustup couldn't
# resolve it (e.g. custom target JSON paths don't go through rustup).
# Already-installed is a no-op success in rustup, so we don't need a
# separate "already there" probe.
function Ensure-Target {
    param([string]$Target)
    # Custom target JSON paths are .json files - rustup can't add those.
    # Caller treats them as available-by-spec.
    if ($Target -match '\.json$') { return $true }
    $r = Invoke-Capture -Cmd "rustup" `
        -CmdArgs @("target","add",$Target,"--toolchain","nightly") `
        -WorkingDir $repoRoot
    # rustup exits 0 on already-installed too.
    return ($r.Exit -eq 0)
}

function Write-TargetHeader {
    param([int]$Index, [int]$Total, [string]$Name)
    Write-Host ""
    Write-Host ("=== [{0}/{1}] {2} ===" -f $Index, $Total, $Name) -ForegroundColor Cyan
}

function Write-TargetResult {
    param(
        [int]$Index,
        [int]$Total,
        [string]$Name,
        [string]$Status,
        [int]$Warnings,
        [int]$Errors,
        [double]$Elapsed,
        [string]$Reason
    )
    $color = switch ($Status) {
        "PASS" { "Green" }
        "FAIL" { "Red" }
        "SKIP" { "Yellow" }
        default { "White" }
    }
    $detail = switch ($Status) {
        "PASS" { "(W: {0}, {1:N0}s)" -f $Warnings, $Elapsed }
        "FAIL" { "(E: {0}, W: {1}, {2:N0}s)" -f $Errors, $Warnings, $Elapsed }
        "SKIP" { "({0})" -f $Reason }
        default { "" }
    }
    Write-Host ("[{0}/{1}] {2}... {3} {4}" -f $Index, $Total, $Name, $Status, $detail) -ForegroundColor $color
}

# ---------------------------------------------------------------------------
# Target matrix
# ---------------------------------------------------------------------------
# Each entry drives one cargo +nightly check invocation. Fields:
#   Key       - short stable id used in summary table (no spaces)
#   Name      - human-readable label printed in [N/M] header line
#   Target    - --target value (triple or path to .json spec); empty for host
#   WorkDir   - subdir cargo runs from (relative to repoRoot)
#   ExtraArgs - extra cargo args (features, -p selectors, -Z flags)
#   AllowSkip - if Ensure-Target fails for this target AND AllowSkip is true,
#               record as SKIP rather than FAIL. Used for the armv7 custom
#               target which rustup can't resolve via `target add`.
#   NoTests   - if true, drop `--tests` from the cargo invocation. Required
#               for the armv7 -Z build-std path because the synthesized
#               sysroot only contains `core,compiler_builtins,alloc` (per
#               Dockerfile.uefi-armv7), so the libtest crate isn't
#               available and `--tests` errors with E0463 ("can't find
#               crate for `test`"). The plain `check` still catches all
#               the silent-break failure modes #441/#452/#654 surfaced;
#               the libtest harness adds nothing on a no_std target that
#               can't link std anyway.
$targets = @(
    @{
        # Host check covers both crates in one cargo invocation. The repo
        # has no top-level workspace Cargo.toml (each crate is its own
        # project), so `-p arest -p arest-kernel` only resolves when run
        # from a directory whose Cargo.toml carries the other as a path
        # dep. arest-kernel/Cargo.toml has `arest = { path = "../arest" }`,
        # so cargo from `crates/arest-kernel` can see both packages and
        # check them in one pass. Explicit `--target x86_64-pc-windows-msvc`
        # overrides the kernel crate's `.cargo/config.toml` default of
        # `x86_64-unknown-uefi`.
        Key       = "x86_64-host"
        Name      = "x86_64-pc-windows-msvc (host)"
        Target    = "x86_64-pc-windows-msvc"
        WorkDir   = "crates\arest-kernel"
        ExtraArgs = @("-p","arest","-p","arest-kernel")
        AllowSkip = $false
    },
    @{
        Key       = "x86_64-uefi-server"
        Name      = "x86_64-unknown-uefi server,static-ip"
        Target    = "x86_64-unknown-uefi"
        WorkDir   = "crates\arest-kernel"
        ExtraArgs = @("--no-default-features","--features","server,static-ip")
        AllowSkip = $false
    },
    @{
        Key       = "x86_64-uefi-default"
        Name      = "x86_64-unknown-uefi default"
        Target    = "x86_64-unknown-uefi"
        WorkDir   = "crates\arest-kernel"
        ExtraArgs = @()
        AllowSkip = $false
    },
    @{
        Key       = "aarch64-uefi-server"
        Name      = "aarch64-unknown-uefi server,static-ip"
        Target    = "aarch64-unknown-uefi"
        WorkDir   = "crates\arest-kernel"
        ExtraArgs = @("--no-default-features","--features","server,static-ip")
        AllowSkip = $false
    },
    @{
        # armv7 uses a custom target JSON sibling to crates/arest-kernel/.cargo/
        # (per Track I #386). rustup has no built-in armv7-unknown-uefi
        # triple, so the spec must be provided as a relative path resolved
        # from the kernel crate dir. The build needs `-Z build-std` to
        # synthesize core/compiler_builtins/alloc - matching the
        # Dockerfile.uefi-armv7 invocation.
        Key       = "armv7-uefi-server"
        Name      = "arm-unknown-uefi server,static-ip"
        Target    = "./arest-kernel-armv7-uefi.json"
        WorkDir   = "crates\arest-kernel"
        ExtraArgs = @(
            "--no-default-features","--features","server,static-ip",
            "-Z","build-std=core,compiler_builtins,alloc",
            "-Z","unstable-options",
            "-Z","json-target-spec"
        )
        AllowSkip = $true
        NoTests   = $true
    },
    @{
        # wasm32 build is the cloudflare worker target — pure arest crate,
        # no kernel. Run from `crates/arest` directly since there's no
        # top-level workspace and `-p arest` only resolves where arest is
        # the local package or a path dep.
        Key       = "wasm32-cloudflare"
        Name      = "wasm32-unknown-unknown cloudflare"
        Target    = "wasm32-unknown-unknown"
        WorkDir   = "crates\arest"
        ExtraArgs = @("--no-default-features","--features","cloudflare,debug-def","-p","arest")
        AllowSkip = $false
    }
)

$totalTargets = $targets.Count

# ---------------------------------------------------------------------------
# Per-target sweep
# ---------------------------------------------------------------------------
$idx = 0
foreach ($t in $targets) {
    $idx++
    Write-TargetHeader $idx $totalTargets $t.Name

    # Try to install the target. SKIP if the triple isn't resolvable
    # AND the entry permits skipping (custom JSON specs).
    $available = Ensure-Target -Target $t.Target
    if (-not $available) {
        if ($t.AllowSkip) {
            Write-TargetResult $idx $totalTargets $t.Name "SKIP" 0 0 0.0 "rustup can't resolve target"
            Add-TargetResult $t.Key $t.Name "SKIP" 0 0 0.0 "rustup can't resolve target" ""
            continue
        } else {
            # Don't bail - the cargo check below may still find the target
            # in some other way (sysroot, custom config). If cargo also
            # fails, we'll record a FAIL with the cargo output.
            Write-Host "  rustup target add failed; proceeding to cargo check anyway." -ForegroundColor Yellow
        }
    }

    $workDir = if ($t.WorkDir -eq ".") { $repoRoot } else { Join-Path $repoRoot $t.WorkDir }

    # Most targets get `cargo check --tests` so integration test files
    # are pulled into the type-check pass too. NoTests entries (armv7,
    # which uses -Z build-std without `test` in the synthesized sysroot)
    # drop `--tests` because libtest isn't available; plain `check` still
    # catches every silent-break failure mode the script exists to surface.
    $checkArgs = if ($t.ContainsKey("NoTests") -and $t.NoTests) { @("check") } else { @("check","--tests") }
    $cargoArgs = @("+nightly") + $checkArgs + @("--target",$t.Target) + $t.ExtraArgs

    # 15-min cap per target. Cold compile of arest-kernel against
    # x86_64-unknown-uefi default (slint runtime + slint-build host
    # codegen) is the slowest entry; observed wall-clock around 6-8 min
    # on a fresh target dir, so 15 min is a comfortable margin.
    $r = Invoke-Capture -Cmd "cargo" `
        -CmdArgs $cargoArgs `
        -WorkingDir $workDir `
        -TimeoutSec 900

    $diag = Count-Diagnostics -Output $r.Output

    if ($r.Exit -eq 0) {
        Write-TargetResult $idx $totalTargets $t.Name "PASS" $diag.Warnings 0 $r.Elapsed ""
        Add-TargetResult $t.Key $t.Name "PASS" $diag.Warnings 0 $r.Elapsed "" $r.Output
    } else {
        # If cargo reported "error: failed to run `rustc` to learn about
        # target-specific information" or similar resolution failure AND
        # this target permits skipping, treat as SKIP. Otherwise FAIL.
        $resolveFail = $false
        if ($t.AllowSkip) {
            if ($r.Output -match "is not a valid (?:target|builtin target)" `
                -or $r.Output -match "Could not find specification for target" `
                -or $r.Output -match "couldn't load target specification") {
                $resolveFail = $true
            }
        }
        if ($resolveFail) {
            Write-TargetResult $idx $totalTargets $t.Name "SKIP" 0 0 $r.Elapsed "target spec not loadable"
            Add-TargetResult $t.Key $t.Name "SKIP" 0 0 $r.Elapsed "target spec not loadable" $r.Output
        } else {
            # If cargo died without emitting any "error:" line at column 0
            # (e.g., link failure, exit code only), still record at least 1
            # error so the summary doesn't claim "E: 0" on a failed run.
            $errs = if ($diag.Errors -gt 0) { $diag.Errors } else { 1 }
            Write-TargetResult $idx $totalTargets $t.Name "FAIL" $diag.Warnings $errs $r.Elapsed ""
            Add-TargetResult $t.Key $t.Name "FAIL" $diag.Warnings $errs $r.Elapsed "" $r.Output
        }
    }
    if ($VerboseOutput) { Write-Host $r.Output }
}

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "=== SUMMARY ===" -ForegroundColor Cyan
$passCount = 0
$failCount = 0
$skipCount = 0
foreach ($s in $results) {
    $color = switch ($s.Status) {
        "PASS" { "Green" }
        "FAIL" { "Red" }
        "SKIP" { "Yellow" }
        default { "White" }
    }
    $detail = switch ($s.Status) {
        "PASS" { "W: {0}, {1:N0}s" -f $s.Warnings, $s.Elapsed }
        "FAIL" { "E: {0}, W: {1}, {2:N0}s" -f $s.Errors, $s.Warnings, $s.Elapsed }
        "SKIP" { $s.Reason }
        default { "" }
    }
    Write-Host ("  {0,-22} {1,-5}  {2}  ({3})" -f $s.Key, $s.Status, $s.Name, $detail) -ForegroundColor $color
    switch ($s.Status) {
        "PASS" { $passCount++ }
        "FAIL" { $failCount++ }
        "SKIP" { $skipCount++ }
    }
}
Write-Host ""
Write-Host ("  TOTAL: {0}/{1} PASS, {2} FAIL, {3} SKIP." `
    -f $passCount, $totalTargets, $failCount, $skipCount)

# Exit code: 0 if no FAILs (skips don't gate); 1 otherwise.
if ($failCount -gt 0) {
    Write-Host "  Exit: 1 (target failures present)." -ForegroundColor Red
    exit 1
}
Write-Host "  Exit: 0 (all targets passed or skipped cleanly)." -ForegroundColor Green
exit 0
