<#
.SYNOPSIS
  Phase 3 system-check gates on the real app (gate build): quick check under emu-low, emu-mid and native, one full
  check under emu-low, and the fingerprint-change hook.

.DESCRIPTION
  Starts target\gate\stelmar-voice-workbench.exe (cargo feature gate-hooks; never an installer build) inside the
  emulation job (wb-emulate --profile <p>) with WB_GATE_CHECK=quick|full. The app runs its own system check through
  the code the System screen uses, saves capability.json into a data folder of its own (WB_GATE_DATA_DIR, so the
  real profile is never touched), writes a JSON report (WB_GATE_REPORT) and exits. The script waits with a time
  limit, then scripts\gates\system_check_report.py writes the evidence: tier, configuration, check time against the
  8-minute bar, hybrid-rule consistency, the measurement table, quick-versus-full RTFs (flag above 10%, deciding
  cells marked) and the fingerprint hook.

  Runs, in this order (a quick and a full check each take the machine for minutes; nothing else may run):
    quick-emu-low, quick-emu-mid, quick-native, full-emu-low,
    view-same-emu-low (reads the profile saved by quick-emu-low: no fingerprint change expected),
    view-changed-emu-low (the same with WB_TEST_FINGERPRINT_CHANGE set: a change expected).

  Each run holds D:\Projects\stelmar-gate.lock (Track B pauses), refuses to start while RTSS, MSI Afterburner or
  LatencyMon run or an engine build is in progress, and waits for the request flag. The machine must be quiet:
  no browser video, no builds, no other gate. The evidence goes through scripts\lib\Redact.psm1.

  Needs the gate build (scripts\gates\build-gate.ps1), the built engines in src-tauri\binaries
  (scripts\build-engines.ps1), the models under models\ and resources\bench. No administrator rights, no audio
  rig (the recorder safety run records from the default input; keep the cable and other inputs quiet).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\system-check-gate.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\system-check-gate.ps1 -Runs quick-emu-low
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\system-check-gate.ps1 -ReportOnly
#>
[CmdletBinding()]
param(
  # With -File a comma list arrives as one string: it is split below.
  [string[]]$Runs = @('quick-emu-low', 'quick-emu-mid', 'quick-native', 'full-emu-low', 'start-during-check-emu-low', 'view-same-emu-low', 'view-changed-emu-low'),
  [string]$Machine = 'Legion',
  [string]$WorkRoot = 'D:\STELMAR-Gate\system-check',
  [string]$OutDir,
  [string]$Exe,
  [string]$Emulate,
  [int]$QuickLimitMinutes = 25,
  [int]$FullLimitMinutes = 90,
  # Write the evidence again from the reports already in -WorkRoot; start nothing.
  [switch]$ReportOnly
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-3' }
if (-not $Exe) { $Exe = Join-Path $repo 'target\gate\stelmar-voice-workbench.exe' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
$allRuns = @('quick-emu-low', 'quick-emu-mid', 'quick-native', 'full-emu-low', 'start-during-check-emu-low', 'view-same-emu-low', 'view-changed-emu-low')
$Runs = @($Runs | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
foreach ($r in $Runs) { if ($allRuns -notcontains $r) { throw "unknown run '$r'; choose from: $($allRuns -join ', ')" } }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\AppProcess.psm1') -Force
New-Item -ItemType Directory -Force -Path $WorkRoot, $OutDir | Out-Null
$python = Join-Path $repo 'eval\.venv\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $python)) { throw 'the evaluation environment is missing: uv sync --project eval' }

function Q([string]$v) { ConvertTo-WbArg $v }

function Invoke-OneRun([string]$Name) {
  $parts = $Name -split '-', 2            # quick|full|view  -  <same|changed-><profile> / <profile>
  $mode = $parts[0]
  $rest = $parts[1]
  $changed = $false
  # Start pressed during a quick check: the check is cancelled, the recording starts, the profile is partial.
  $pressStart = $false
  if ($Name -eq 'start-during-check-emu-low') { $mode = 'quick'; $rest = 'emu-low'; $pressStart = $true }
  if ($mode -eq 'view') {
    $changed = $rest.StartsWith('changed-')
    $rest = $rest -replace '^(same|changed)-', ''
  }
  $profile = $rest                        # emu-low, emu-mid, native
  $run = Join-Path $WorkRoot $Name
  if (Test-Path -LiteralPath $run) { Remove-Item -LiteralPath $run -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $run | Out-Null
  # A view run reads the profile the quick emu-low run saved; every check run starts with an empty data folder.
  $data = if ($mode -eq 'view') { Join-Path $WorkRoot 'quick-emu-low\data' } else { Join-Path $run 'data' }
  if ($mode -ne 'view') { New-Item -ItemType Directory -Force -Path $data | Out-Null }
  elseif (-not (Test-Path -LiteralPath (Join-Path $data 'capability.json'))) { throw "no saved profile at $data; run quick-emu-low first" }
  $limit = if ($mode -eq 'full') { $FullLimitMinutes } elseif ($mode -eq 'quick') { $QuickLimitMinutes } else { 5 }
  $report = Join-Path $run 'report.json'
  $env:WB_GATE_CHECK = $mode
  $env:WB_GATE_DATA_DIR = $data
  $env:WB_GATE_REPORT = $report
  $env:WB_GATE_ROOT = Join-Path $run 'rec'
  $env:WB_BENCH_DIR = Join-Path $repo 'resources\bench'
  if ($pressStart) { $env:WB_GATE_START_DURING_CHECK = '60' }
  if ($changed) { $env:WB_TEST_FINGERPRINT_CHANGE = 'gate' } else { Remove-Item Env:\WB_TEST_FINGERPRINT_CHANGE -ErrorAction SilentlyContinue }
  try {
    Invoke-WbGate -Name "system-check $Name" -Body {
      $args2 = @('--profile', $profile, '--log', (Q (Join-Path $run 'emulate.log')), '--', (Q $Exe))
      $started = Get-Date
      $p = Start-Process -FilePath $Emulate -ArgumentList $args2 -PassThru -WindowStyle Hidden
      $deadline = $started.AddMinutes($limit)
      while (-not $p.HasExited) {
        if ((Get-Date) -gt $deadline) {
          [void](Stop-WbApp)
          Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
          throw "the $Name run did not finish within $limit minutes; stopped it"
        }
        Start-Sleep -Seconds 2
      }
      $script:exitCode = $p.ExitCode
      Write-Host ("{0}: exit {1} after {2:N1} min" -f $Name, $p.ExitCode, ((Get-Date) - $started).TotalMinutes)
    } | Out-Null
  } finally {
    foreach ($v in 'WB_GATE_START_DURING_CHECK', 'WB_GATE_CHECK', 'WB_GATE_DATA_DIR', 'WB_GATE_REPORT', 'WB_GATE_ROOT', 'WB_BENCH_DIR', 'WB_TEST_FINGERPRINT_CHANGE') {
      Remove-Item "Env:\$v" -ErrorAction SilentlyContinue
    }
  }
  if (-not (Test-Path -LiteralPath $report)) { throw "$Name wrote no report (exit $script:exitCode); see $run\emulate.log and the app log" }
}

$failures = @()
if (-not $ReportOnly) {
  foreach ($f in @($Exe, $Emulate)) { if (-not (Test-Path -LiteralPath $f)) { throw "Missing $f; build it first (scripts\gates\build-gate.ps1)" } }
  $overlay = @(Get-WbOverlayTools)
  if ($overlay.Count) { throw "refusing to run: overlay tools are running: $($overlay -join ', ')" }
  foreach ($r in $Runs) {
    Write-Host "$(Get-Date -Format 'HH:mm:ss') starting $r"
    try { Invoke-OneRun $r } catch { Write-Host "FAILED $r : $($_.Exception.Message)"; $failures += @("$r : $($_.Exception.Message)") }
  }
}

$tmp = Join-Path $WorkRoot 'report.txt'
$rep = Invoke-WbNative -FilePath $python -ArgumentList @('-X', 'utf8', (Join-Path $repo 'scripts\gates\system_check_report.py'), $WorkRoot, $tmp)
if (-not (Test-Path -LiteralPath $tmp)) { throw "the report was not written: $($rep.Error -join ' ')" }
$body = @([IO.File]::ReadAllLines($tmp, [Text.Encoding]::UTF8))
$header = @(
  "date: $(Get-Date -Format 'yyyy-MM-dd HH:mm') (IST on the Legion)",
  "command: powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\system-check-gate.ps1" + $(if ($ReportOnly) { ' -ReportOnly' } else { " -Runs $($Runs -join ',')" }),
  "machine: $Machine (the RTX 4070 Laptop GPU and the Intel UHD, emulation per run below); the gate build target\gate\stelmar-voice-workbench.exe runs the app's own system check through the WB_GATE_CHECK hook",
  "source: $((Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $repo, 'rev-parse', '--short', 'HEAD')).Output -join '')",
  "build: $(Split-Path -Leaf $Exe) sha256 $((Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash.ToLowerInvariant())"
)
if ($failures.Count) { $header += @($failures | ForEach-Object { "run problem: $_" }) }
Write-WbEvidence -Path (Join-Path $OutDir 'system-check-gate.txt') -Lines ($header + $body)
Write-Host "evidence: $(Join-Path $OutDir 'system-check-gate.txt')"
if ($failures.Count) { exit 1 }
