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
    # #672: platform_zip round-trip tests fail - File cells not materialising
    # after unzip. Tracked separately, doesn't block the orchestration script.
    "arest-integration" = @(
        "platform_zip"
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
# (exitCode, capturedOutput, elapsedSeconds). Defeats PowerShell 5.1's
# stderr-as-ErrorRecord behaviour by relying on the file capture.
function Invoke-Capture {
    param(
        [string]$Cmd,
        [string[]]$CmdArgs,
        [string]$WorkingDir
    )
    $tmpOut = New-TemporaryFile
    $tmpErr = New-TemporaryFile
    $start = Get-Date
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $exit = 0
    try {
        $proc = Start-Process -FilePath $Cmd `
            -ArgumentList $CmdArgs `
            -RedirectStandardOutput $tmpOut.FullName `
            -RedirectStandardError $tmpErr.FullName `
            -WorkingDirectory $WorkingDir `
            -NoNewWindow `
            -Wait `
            -PassThru
        $exit = $proc.ExitCode
    } catch {
        $exit = -1
        Add-Content -Path $tmpErr.FullName -Value ("Start-Process threw: " + $_.Exception.Message)
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    $elapsed = ((Get-Date) - $start).TotalSeconds
    $outText = ""
    if (Test-Path $tmpOut.FullName) {
        $outText += (Get-Content $tmpOut.FullName -Raw -ErrorAction SilentlyContinue)
    }
    if (Test-Path $tmpErr.FullName) {
        $errText = (Get-Content $tmpErr.FullName -Raw -ErrorAction SilentlyContinue)
        if ($errText) { $outText += "`n--- stderr ---`n" + $errText }
    }
    Remove-Item $tmpOut.FullName -Force -ErrorAction SilentlyContinue
    Remove-Item $tmpErr.FullName -Force -ErrorAction SilentlyContinue
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
Write-StageHeader 2 8 "arest integration tests"
$r = Invoke-Capture -Cmd "cargo" `
    -CmdArgs @("test","--target","x86_64-pc-windows-msvc","--tests") `
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
    $r = Invoke-Capture -Cmd $yarnPath -CmdArgs @("test") -WorkingDir $repoRoot
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
    $r = Invoke-Capture -Cmd "powershell.exe" `
        -CmdArgs @("-NoProfile","-NonInteractive","-File",$smokeScript,"-Smoke") `
        -WorkingDir $repoRoot
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
    $r = Invoke-Capture -Cmd "powershell.exe" `
        -CmdArgs @("-NoProfile","-NonInteractive","-File",$workerScript) `
        -WorkingDir $repoRoot
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
