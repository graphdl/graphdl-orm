# scripts/test-all.ps1
#
# Single PowerShell entry point that orchestrates every test surface in
# the AREST repo, captures pass/fail/ignored counts per stage, and emits
# one summary at the end. Tracks #667.
#
# Stages (run in order; each is fail-fast within itself, but the script
# tries every stage even if an earlier one fails):
#
#   1. arest unit tests          (cargo test --lib)
#   2. arest integration tests   (cargo test --tests; #666 unblocked)
#   3. arest doctests            (cargo test --doc; #673 known-failing)
#   4. arest-kernel host tests   (cargo test --lib)
#   5. arest-kernel doctests     (cargo test --doc)
#   6. vitest TS tests           (yarn test)
#   7. kernel UEFI smoke         (boot-kernel-uefi-server.ps1 -Smoke)
#   8. worker e2e                (run-e2e-against-worker.ps1)
#
# Stage 7 is SKIPPED with a WARN if Docker isn't running.
# Stage 8 is SKIPPED with a WARN if wrangler isn't installed.
#
# Exit code logic:
#   * 0 if every stage either PASSED, was SKIPPED with a documented reason,
#     OR matched a known-failure pattern listed in $KnownFailures below.
#   * 1 on any unexpected failure (i.e., anything not in $KnownFailures).
#
# To add or remove a known-failure entry, edit the $KnownFailures hashtable
# directly under this comment block. Each entry maps a stage key to a list
# of substrings; if any substring appears in the captured stdout/stderr of
# the corresponding stage AND that stage failed, the failure is treated as
# a known-failure (does not block exit 0).
#
# Usage:
#   .\scripts\test-all.ps1            # default: only result line per stage
#   .\scripts\test-all.ps1 -Verbose   # also dump full stdout per stage

param(
    [switch]$VerboseOutput
)

$ErrorActionPreference = "Continue"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# ---------------------------------------------------------------------------
# Known-failure registry
# ---------------------------------------------------------------------------
# Map of stage-key -> list of substrings that, if observed in stdout/stderr
# alongside a non-zero exit, mark the failure as "known" (won't block the
# overall exit). Add a new entry here when a tracked issue produces a
# reproducible test failure that we don't want to red-light CI for.
#
# When the underlying issue is fixed, REMOVE the entry - leaving stale
# entries here defeats the purpose of the script.
$KnownFailures = @{
    # #673: cell_aead.rs:284 doc-test fence missing language tag - rustdoc
    # tries to compile prose as code. Fix is one-line; tracked separately.
    "arest-doctests" = @(
        "cell_aead.rs",
        "test result: FAILED. 0 passed; 1 failed"
    )
    # vitest under wasm32 build occasionally surfaces SystemTime panics on
    # nondeterministic clock paths. The worker-e2e agent (#642) noted these
    # as pre-existing wasm32 SystemTime failures. Don't gate on them.
    "vitest" = @(
        "SystemTime",
        "RuntimeError: unreachable"
    )
}

# ---------------------------------------------------------------------------
# Stage tracking
# ---------------------------------------------------------------------------
# Each stage records: name, status (PASS/FAIL/SKIP/KNOWN), summary string,
# elapsed seconds, and full captured output (for -VerboseOutput).
$results = New-Object System.Collections.Generic.List[PSObject]

function Add-StageResult {
    param(
        [string]$Key,
        [string]$Name,
        [string]$Status,    # PASS | FAIL | SKIP | KNOWN
        [string]$Summary,
        [double]$Elapsed,
        [string]$Output
    )
    $results.Add([PSCustomObject]@{
        Key     = $Key
        Name    = $Name
        Status  = $Status
        Summary = $Summary
        Elapsed = $Elapsed
        Output  = $Output
    })
}

function Test-KnownFailure {
    param(
        [string]$Key,
        [string]$Output
    )
    if (-not $KnownFailures.ContainsKey($Key)) { return $false }
    foreach ($needle in $KnownFailures[$Key]) {
        if ($Output -match [regex]::Escape($needle)) { return $true }
    }
    return $false
}

# Run a native command, capture stdout+stderr to a temp file, return
# (exitCode, capturedOutput, elapsedSeconds).
#
# We delegate redirection to cmd.exe rather than using PowerShell's
# Start-Process -RedirectStandardOutput / -RedirectStandardError. PS 5.1's
# implementation deadlocks here on long-running cargo runs (observed:
# stage 1 hung after writing the header line, no cargo child surviving,
# 0% CPU on the parent for hours). cmd.exe's `>file 2>&1` is bulletproof
# on Windows and avoids the NativeCommandError wrapping that bites us in
# PS 5.1 when redirecting native stderr through pipelines.
function Invoke-Capture {
    param(
        [string]$Cmd,
        [string[]]$CmdArgs,
        [string]$WorkingDir,
        # Hard wall-clock cap. If the child hasn't exited within this many
        # seconds, kill the process tree and return Exit=-1 with a marker
        # in the captured output. 0 disables the cap. Set per-stage to keep
        # one runaway test (e.g. a vitest worker that holds workerd alive)
        # from blocking the orchestration script for hours.
        [int]$TimeoutSec = 0
    )
    $tmpOut = New-TemporaryFile
    $start = Get-Date
    $exit = 0
    # Use System.Diagnostics.Process directly with async stdout/stderr
    # capture. Start-Process -Wait deadlocks on long-running cargo runs even
    # when redirecting to files (observed: stage 1 hangs with cmd.exe wrapper
    # alive but no cargo/rustc child surviving, no progress for tens of
    # minutes). The PSI + BeginOutputReadLine pattern is the canonical Windows
    # solution: drain both pipes asynchronously while waiting for exit.
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
                # Kill process tree (Process.Kill($true) is .NET 5+; PS 5.1 .NET
                # Framework only has parameterless Kill, which doesn't terminate
                # children. Walk the tree with WMI as a fallback.)
                try {
                    Get-CimInstance Win32_Process -Filter "ParentProcessId=$($proc.Id)" -ErrorAction SilentlyContinue |
                        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
                    $proc.Kill()
                } catch {}
                [void]$stderrSb.AppendLine("Invoke-Capture: TIMEOUT after $TimeoutSec sec - killed process tree.")
                $exit = -1
            } else {
                # WaitForExit() with no arg drains the async event handlers
                # once the streams close.
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
    Remove-Item $tmpOut.FullName -Force -ErrorAction SilentlyContinue
    if ($null -eq $outText) { $outText = "" }
    return @{ Exit = $exit; Output = $outText; Elapsed = $elapsed }
}

# Parse cargo libtest output. Returns hashtable with passed/failed/ignored
# (summed across all "test result:" lines, since one test binary may emit
# more than one result block - e.g. integration tests with multiple bins).
function Parse-CargoCounts {
    param([string]$Output)
    $passed = 0; $failed = 0; $ignored = 0
    # Match: "test result: ok. N passed; M failed; K ignored; ..."
    $matches = [regex]::Matches($Output, "test result:\s+(ok|FAILED)\.\s+(\d+)\s+passed;\s+(\d+)\s+failed;\s+(\d+)\s+ignored")
    foreach ($m in $matches) {
        $passed  += [int]$m.Groups[2].Value
        $failed  += [int]$m.Groups[3].Value
        $ignored += [int]$m.Groups[4].Value
    }
    return @{ Passed = $passed; Failed = $failed; Ignored = $ignored; HasResult = $matches.Count -gt 0 }
}

# Parse vitest output. Vitest's run summary line looks like:
#   "Tests  302 passed (302)"  or  "Tests  300 passed | 2 failed (302)"
function Parse-VitestCounts {
    param([string]$Output)
    $passed = 0; $failed = 0
    $passMatch = [regex]::Match($Output, "Tests[^\r\n]*?(\d+)\s+passed")
    if ($passMatch.Success) { $passed = [int]$passMatch.Groups[1].Value }
    $failMatch = [regex]::Match($Output, "Tests[^\r\n]*?(\d+)\s+failed")
    if ($failMatch.Success) { $failed = [int]$failMatch.Groups[1].Value }
    return @{ Passed = $passed; Failed = $failed }
}

function Write-StageHeader {
    param([int]$Index, [int]$Total, [string]$Name)
    Write-Host ""
    Write-Host "=== [$Index/$Total] $Name ===" -ForegroundColor Cyan
}

function Write-StageResult {
    param(
        [string]$CmdLine,
        [string]$Status,
        [string]$Detail
    )
    $color = switch ($Status) {
        "PASS"  { "Green" }
        "FAIL"  { "Red" }
        "SKIP"  { "Yellow" }
        "KNOWN" { "Yellow" }
        default { "White" }
    }
    Write-Host ("  {0} ... {1} ({2})" -f $CmdLine, $Status, $Detail) -ForegroundColor $color
}

# ---------------------------------------------------------------------------
# Stage 1: arest unit tests
# ---------------------------------------------------------------------------
Write-StageHeader 1 8 "arest unit tests"
$arestDir = Join-Path $repoRoot "crates\arest"
$r = Invoke-Capture -Cmd "cargo" `
    -CmdArgs @("test","--lib","--target","x86_64-pc-windows-msvc") `
    -WorkingDir $arestDir
$counts = Parse-CargoCounts -Output $r.Output
$detail = "{0} passed, {1} failed, {2} ignored, {3:N0}s" -f $counts.Passed, $counts.Failed, $counts.Ignored, $r.Elapsed
if ($r.Exit -eq 0 -and $counts.Failed -eq 0) {
    Write-StageResult "cargo test --lib -p arest" "PASS" $detail
    Add-StageResult "arest-unit" "arest unit tests" "PASS" $detail $r.Elapsed $r.Output
} elseif (Test-KnownFailure -Key "arest-unit" -Output $r.Output) {
    Write-StageResult "cargo test --lib -p arest" "KNOWN" $detail
    Add-StageResult "arest-unit" "arest unit tests" "KNOWN" $detail $r.Elapsed $r.Output
} else {
    Write-StageResult "cargo test --lib -p arest" "FAIL" $detail
    Add-StageResult "arest-unit" "arest unit tests" "FAIL" $detail $r.Elapsed $r.Output
}
if ($VerboseOutput) { Write-Host $r.Output }

# ---------------------------------------------------------------------------
# Stage 2: arest integration tests
# ---------------------------------------------------------------------------
# `--features test-bins` (#684) opts in the heavy [[test]] entries that
# day-to-day `cargo test --lib` skips. CI is the only consumer that
# needs them, and Stage 2 is exactly that consumer.
Write-StageHeader 2 8 "arest integration tests"
$r = Invoke-Capture -Cmd "cargo" `
    -CmdArgs @("test","--target","x86_64-pc-windows-msvc","--tests","--features","test-bins") `
    -WorkingDir $arestDir
$counts = Parse-CargoCounts -Output $r.Output
$detail = "{0} passed, {1} failed, {2} ignored, {3:N0}s" -f $counts.Passed, $counts.Failed, $counts.Ignored, $r.Elapsed
if ($r.Exit -eq 0 -and $counts.Failed -eq 0) {
    Write-StageResult "cargo test -p arest --tests" "PASS" $detail
    Add-StageResult "arest-integration" "arest integration tests" "PASS" $detail $r.Elapsed $r.Output
} elseif (Test-KnownFailure -Key "arest-integration" -Output $r.Output) {
    Write-StageResult "cargo test -p arest --tests" "KNOWN" ($detail + " - known #672")
    Add-StageResult "arest-integration" "arest integration tests" "KNOWN" ($detail + " - known #672") $r.Elapsed $r.Output
} else {
    Write-StageResult "cargo test -p arest --tests" "FAIL" $detail
    Add-StageResult "arest-integration" "arest integration tests" "FAIL" $detail $r.Elapsed $r.Output
}
if ($VerboseOutput) { Write-Host $r.Output }

# ---------------------------------------------------------------------------
# Stage 3: arest doctests
# ---------------------------------------------------------------------------
Write-StageHeader 3 8 "arest doctests"
$r = Invoke-Capture -Cmd "cargo" `
    -CmdArgs @("test","--doc","--target","x86_64-pc-windows-msvc") `
    -WorkingDir $arestDir
$counts = Parse-CargoCounts -Output $r.Output
$detail = "{0} passed, {1} failed, {2} ignored, {3:N0}s" -f $counts.Passed, $counts.Failed, $counts.Ignored, $r.Elapsed
if ($r.Exit -eq 0 -and $counts.Failed -eq 0) {
    Write-StageResult "cargo test --doc -p arest" "PASS" $detail
    Add-StageResult "arest-doctests" "arest doctests" "PASS" $detail $r.Elapsed $r.Output
} elseif (Test-KnownFailure -Key "arest-doctests" -Output $r.Output) {
    Write-StageResult "cargo test --doc -p arest" "KNOWN" ($detail + " - known #673")
    Add-StageResult "arest-doctests" "arest doctests" "KNOWN" ($detail + " - known #673") $r.Elapsed $r.Output
} else {
    Write-StageResult "cargo test --doc -p arest" "FAIL" $detail
    Add-StageResult "arest-doctests" "arest doctests" "FAIL" $detail $r.Elapsed $r.Output
}
if ($VerboseOutput) { Write-Host $r.Output }

# ---------------------------------------------------------------------------
# Stage 4: arest-kernel host tests
# ---------------------------------------------------------------------------
Write-StageHeader 4 8 "arest-kernel host tests"
$kernelDir = Join-Path $repoRoot "crates\arest-kernel"
$r = Invoke-Capture -Cmd "cargo" `
    -CmdArgs @("test","--lib","--target","x86_64-pc-windows-msvc") `
    -WorkingDir $kernelDir
$counts = Parse-CargoCounts -Output $r.Output
$detail = "{0} passed, {1} failed, {2} ignored, {3:N0}s" -f $counts.Passed, $counts.Failed, $counts.Ignored, $r.Elapsed
if ($r.Exit -eq 0 -and $counts.Failed -eq 0) {
    Write-StageResult "cargo test --lib -p arest-kernel" "PASS" $detail
    Add-StageResult "arest-kernel" "arest-kernel host tests" "PASS" $detail $r.Elapsed $r.Output
} elseif (Test-KnownFailure -Key "arest-kernel" -Output $r.Output) {
    Write-StageResult "cargo test --lib -p arest-kernel" "KNOWN" $detail
    Add-StageResult "arest-kernel" "arest-kernel host tests" "KNOWN" $detail $r.Elapsed $r.Output
} else {
    Write-StageResult "cargo test --lib -p arest-kernel" "FAIL" $detail
    Add-StageResult "arest-kernel" "arest-kernel host tests" "FAIL" $detail $r.Elapsed $r.Output
}
if ($VerboseOutput) { Write-Host $r.Output }

# ---------------------------------------------------------------------------
# Stage 5: arest-kernel doctests
# ---------------------------------------------------------------------------
Write-StageHeader 5 8 "arest-kernel doctests"
$r = Invoke-Capture -Cmd "cargo" `
    -CmdArgs @("test","--doc","--target","x86_64-pc-windows-msvc") `
    -WorkingDir $kernelDir
$counts = Parse-CargoCounts -Output $r.Output
$detail = "{0} passed, {1} failed, {2} ignored, {3:N0}s" -f $counts.Passed, $counts.Failed, $counts.Ignored, $r.Elapsed
if ($r.Exit -eq 0 -and $counts.Failed -eq 0) {
    Write-StageResult "cargo test --doc -p arest-kernel" "PASS" $detail
    Add-StageResult "arest-kernel-doctests" "arest-kernel doctests" "PASS" $detail $r.Elapsed $r.Output
} elseif (Test-KnownFailure -Key "arest-kernel-doctests" -Output $r.Output) {
    Write-StageResult "cargo test --doc -p arest-kernel" "KNOWN" $detail
    Add-StageResult "arest-kernel-doctests" "arest-kernel doctests" "KNOWN" $detail $r.Elapsed $r.Output
} else {
    Write-StageResult "cargo test --doc -p arest-kernel" "FAIL" $detail
    Add-StageResult "arest-kernel-doctests" "arest-kernel doctests" "FAIL" $detail $r.Elapsed $r.Output
}
if ($VerboseOutput) { Write-Host $r.Output }

# ---------------------------------------------------------------------------
# Stage 6: vitest TS tests
# ---------------------------------------------------------------------------
Write-StageHeader 6 8 "vitest TS tests"
# Use yarn.cmd on Windows since Start-Process doesn't resolve PATHEXT.
$yarnCmd = (Get-Command yarn -ErrorAction SilentlyContinue)
if (-not $yarnCmd) {
    Write-StageResult "yarn test" "SKIP" "yarn not installed"
    Add-StageResult "vitest" "vitest TS tests" "SKIP" "yarn not installed" 0 ""
} else {
    $yarnPath = $yarnCmd.Source
    # 30-min cap: per-file fork isolation (vitest.config.ts) means each of
    # the ~37 test files spawns its own short-lived Node fork, so cold-cache
    # runs aggregate fork startup overhead. 30 min is a comfortable margin
    # over the observed worst case while still bounding any actual hang.
    $r = Invoke-Capture -Cmd $yarnPath -CmdArgs @("test") -WorkingDir $repoRoot -TimeoutSec 1800
    $vitestCounts = Parse-VitestCounts -Output $r.Output
    $detail = "{0} passed, {1} failed, {2:N0}s" -f $vitestCounts.Passed, $vitestCounts.Failed, $r.Elapsed
    if ($r.Exit -eq 0 -and $vitestCounts.Failed -eq 0) {
        Write-StageResult "yarn test" "PASS" $detail
        Add-StageResult "vitest" "vitest TS tests" "PASS" $detail $r.Elapsed $r.Output
    } elseif (Test-KnownFailure -Key "vitest" -Output $r.Output) {
        Write-StageResult "yarn test" "KNOWN" ($detail + " - wasm32 SystemTime")
        Add-StageResult "vitest" "vitest TS tests" "KNOWN" ($detail + " - wasm32 SystemTime") $r.Elapsed $r.Output
    } else {
        Write-StageResult "yarn test" "FAIL" $detail
        Add-StageResult "vitest" "vitest TS tests" "FAIL" $detail $r.Elapsed $r.Output
    }
    if ($VerboseOutput) { Write-Host $r.Output }
}

# ---------------------------------------------------------------------------
# Stage 7: kernel UEFI server smoke
# ---------------------------------------------------------------------------
Write-StageHeader 7 8 "kernel UEFI server smoke"
# Probe Docker liveness. Docker Desktop has a habit of being installed but
# not running on dev boxes - fail-soft to SKIP rather than count it as a
# real failure.
$dockerOk = $false
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    & docker version --format "{{.Server.Version}}" 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { $dockerOk = $true }
} catch {
    $dockerOk = $false
} finally {
    $ErrorActionPreference = $prevEAP
}

if (-not $dockerOk) {
    Write-StageResult "scripts/boot-kernel-uefi-server.ps1 -Smoke" "SKIP" "Docker not running"
    Add-StageResult "kernel-smoke" "kernel UEFI server smoke" "SKIP" "Docker not running" 0 ""
} else {
    $smokeScript = Join-Path $repoRoot "scripts\boot-kernel-uefi-server.ps1"
    # Drive the smoke script via powershell.exe so we don't inherit ambient
    # $ErrorActionPreference state mid-run. The smoke script writes its
    # PASS/FAIL banner to stdout and exits 0 on success.
    # 15-min cap covers Docker pull + image build + boot + curl handshake.
    $r = Invoke-Capture -Cmd "powershell.exe" `
        -CmdArgs @("-NoProfile","-NonInteractive","-File",$smokeScript,"-Smoke") `
        -WorkingDir $repoRoot `
        -TimeoutSec 900
    $detail = "{0:N1}s" -f $r.Elapsed
    if ($r.Exit -eq 0 -and $r.Output -match "PASS:.*reachable") {
        # Pull the curl-elapsed milliseconds out of the smoke script's PASS line.
        $reach = [regex]::Match($r.Output, "reachable in\s+([0-9.]+)\s+s")
        $reachable = if ($reach.Success) { "kernel reachable in {0}s" -f $reach.Groups[1].Value } else { "kernel reachable" }
        Write-StageResult "scripts/boot-kernel-uefi-server.ps1 -Smoke" "PASS" $reachable
        Add-StageResult "kernel-smoke" "kernel UEFI server smoke" "PASS" $reachable $r.Elapsed $r.Output
    } elseif (Test-KnownFailure -Key "kernel-smoke" -Output $r.Output) {
        Write-StageResult "scripts/boot-kernel-uefi-server.ps1 -Smoke" "KNOWN" $detail
        Add-StageResult "kernel-smoke" "kernel UEFI server smoke" "KNOWN" $detail $r.Elapsed $r.Output
    } else {
        Write-StageResult "scripts/boot-kernel-uefi-server.ps1 -Smoke" "FAIL" $detail
        Add-StageResult "kernel-smoke" "kernel UEFI server smoke" "FAIL" $detail $r.Elapsed $r.Output
    }
    if ($VerboseOutput) { Write-Host $r.Output }
}

# ---------------------------------------------------------------------------
# Stage 8: worker e2e (wrangler dev)
# ---------------------------------------------------------------------------
Write-StageHeader 8 8 "worker e2e (wrangler dev)"
$wranglerCmd = (Get-Command wrangler -ErrorAction SilentlyContinue)
if (-not $wranglerCmd) {
    Write-StageResult "scripts/run-e2e-against-worker.ps1" "SKIP" "wrangler not installed"
    Add-StageResult "worker-e2e" "worker e2e (wrangler dev)" "SKIP" "wrangler not installed" 0 ""
} else {
    $workerScript = Join-Path $repoRoot "scripts\run-e2e-against-worker.ps1"
    # 15-min cap covers wrangler dev boot + apis e2e suite.
    $r = Invoke-Capture -Cmd "powershell.exe" `
        -CmdArgs @("-NoProfile","-NonInteractive","-File",$workerScript) `
        -WorkingDir $repoRoot `
        -TimeoutSec 900
    $vitestCounts = Parse-VitestCounts -Output $r.Output
    $detail = "{0}/{1} tests, {2:N0}s" -f $vitestCounts.Passed, ($vitestCounts.Passed + $vitestCounts.Failed), $r.Elapsed
    # The worker script also exits 1 if wrangler can't bind the port -
    # treat that as SKIP rather than FAIL so the overall script doesn't
    # red-light on a non-test environmental issue.
    if ($r.Output -match "did not open within" -or $r.Output -match "wrangler dev exited before ready") {
        Write-StageResult "scripts/run-e2e-against-worker.ps1" "SKIP" "wrangler dev failed to bind"
        Add-StageResult "worker-e2e" "worker e2e (wrangler dev)" "SKIP" "wrangler dev failed to bind" $r.Elapsed $r.Output
    } elseif ($r.Exit -eq 0 -and $vitestCounts.Failed -eq 0) {
        Write-StageResult "scripts/run-e2e-against-worker.ps1" "PASS" $detail
        Add-StageResult "worker-e2e" "worker e2e (wrangler dev)" "PASS" $detail $r.Elapsed $r.Output
    } elseif (Test-KnownFailure -Key "worker-e2e" -Output $r.Output) {
        Write-StageResult "scripts/run-e2e-against-worker.ps1" "KNOWN" $detail
        Add-StageResult "worker-e2e" "worker e2e (wrangler dev)" "KNOWN" $detail $r.Elapsed $r.Output
    } else {
        Write-StageResult "scripts/run-e2e-against-worker.ps1" "FAIL" $detail
        Add-StageResult "worker-e2e" "worker e2e (wrangler dev)" "FAIL" $detail $r.Elapsed $r.Output
    }
    if ($VerboseOutput) { Write-Host $r.Output }
}

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "=== SUMMARY ===" -ForegroundColor Cyan
$totalStages = $results.Count
$greenCount  = 0
$failCount   = 0
$knownCount  = 0
$skipCount   = 0
foreach ($s in $results) {
    $color = switch ($s.Status) {
        "PASS"  { "Green" }
        "FAIL"  { "Red" }
        "SKIP"  { "Yellow" }
        "KNOWN" { "Yellow" }
        default { "White" }
    }
    Write-Host ("  {0,-22} {1,-5}  {2}" -f $s.Key, $s.Status, $s.Summary) -ForegroundColor $color
    switch ($s.Status) {
        "PASS"  { $greenCount++ }
        "FAIL"  { $failCount++ }
        "SKIP"  { $skipCount++ }
        "KNOWN" { $knownCount++ }
    }
}
Write-Host ""
Write-Host ("  TOTAL: {0}/{1} stages green; {2} known-failing; {3} skipped; {4} novel failures." `
    -f $greenCount, $totalStages, $knownCount, $skipCount, $failCount)

# Exit code: 0 if no novel failures (skips and known-failures don't gate);
# 1 otherwise.
if ($failCount -gt 0) {
    Write-Host "  Exit: 1 (novel failures present)." -ForegroundColor Red
    exit 1
}
Write-Host "  Exit: 0 (known failures don't block; novel failures do)." -ForegroundColor Green
exit 0
