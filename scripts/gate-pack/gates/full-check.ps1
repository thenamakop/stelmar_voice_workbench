# Gate full-check: the app's full system check through the gate build's WB_GATE_CHECK=full hook (whole bench
# clips, every device, the night model), within the 120 minute limit, then the quick check's saved
# capability.json against the full one on the same rolling cells (a difference over 10% is flagged, and
# marked DECIDING where the cell chose a model). Information for the owner; a check that does not finish,
# or a recorder safety run that fails, is not information. Skipped under -Quick (the runner handles that).
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'GateRun.psm1') -Force
Import-Module (Join-Path $lib 'Redact.psm1') -Force
Import-Module (Join-Path $lib 'AppProcess.psm1') -Force
Import-Module (Join-Path $lib 'ProcWatch.psm1') -Force
Import-Module (Join-Path $lib 'QuickCheck.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'full-check' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $gdir = [string]$ctx.gate_dir
  $run = Join-Path $gdir 'run'
  if (Test-Path -LiteralPath $run) { Remove-Item -LiteralPath $run -Recurse -Force }
  $data = Join-Path $run 'data'
  New-Item -ItemType Directory -Force -Path $data | Out-Null
  $report = Join-Path $run 'report.json'
  foreach ($f in @($paths.GateExe, $paths.BenchClips)) { if (-not (Test-Path -LiteralPath $f)) { throw "the pack is missing $f (run run-gates.ps1 -Verify)" } }
  if ($ctx.profile -ne 'laptop' -and (-not $paths.Emulate -or -not (Test-Path -LiteralPath $paths.Emulate))) { throw "the emulation profile $($ctx.profile) needs wb-emulate; it is not available (dry run on the Legion only)" }

  $env:WB_GATE_CHECK = 'full'
  $env:WB_GATE_DATA_DIR = $data
  $env:WB_GATE_REPORT = $report
  $env:WB_GATE_ROOT = Join-Path $run 'rec'
  $env:WB_BENCH_DIR = $paths.BenchClips
  Remove-Item Env:\WB_TEST_FINGERPRINT_CHANGE -ErrorAction SilentlyContinue
  $script:qcWatch = $null
  $script:qcExit = $null
  $script:qcMinutes = 0.0
  $cells = @()
  $exitCode = $null
  $minutes = 0.0
  try {
    Invoke-WbGate -Name 'full-check' -Body {
      $script:qcWatch = Start-PackProcWatch -Dir (Join-Path $gdir 'watch')
      $started = Get-Date
      if ($ctx.profile -eq 'laptop') {
        $p = Start-Process -FilePath $paths.GateExe -PassThru -WindowStyle Hidden
      } else {
        $a = @('--profile', ([string]$ctx.profile), '--log', (ConvertTo-PackArg (Join-Path $run 'emulate.log')), '--', (ConvertTo-PackArg $paths.GateExe))
        $p = Start-Process -FilePath $paths.Emulate -ArgumentList $a -PassThru -WindowStyle Hidden
      }
      $null = $p.Handle
      $deadline = $started.AddMinutes(120)
      while (-not $p.HasExited) {
        if ((Get-Date) -gt $deadline) {
          [void](Stop-WbApp)
          Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
          throw 'the full check did not finish within 120 minutes; stopped it'
        }
        Start-Sleep -Seconds 2
      }
      $script:qcExit = $p.ExitCode
      $script:qcMinutes = ((Get-Date) - $started).TotalMinutes
    } | Out-Null
    $exitCode = $script:qcExit
    $minutes = $script:qcMinutes
  } finally {
    if ($null -ne $script:qcWatch) { $cells = @(Stop-PackProcWatch -Handle $script:qcWatch) }
    foreach ($v in 'WB_GATE_CHECK', 'WB_GATE_DATA_DIR', 'WB_GATE_REPORT', 'WB_GATE_ROOT', 'WB_BENCH_DIR') { Remove-Item "Env:\$v" -ErrorAction SilentlyContinue }
  }
  Write-PackGateLog -Ctx $ctx -Message ("the gate build exited {0} after {1:N1} min" -f $exitCode, $minutes)

  $rep = Read-PackJson -Path $report
  $cap = Read-PackJson -Path (Join-Path $data 'capability.json')
  $verdict = Get-QuickCheckVerdict -Report $rep -Capability $cap -Cells $cells -BarSeconds 7200
  $quickCap = Read-PackJson -Path (Join-Path ([string]$ctx.work_dir) 'gates\quick-check\capability.json')
  $cmp = Get-QuickFullComparison -Quick $quickCap -Full $cap
  # The excerpt clips are large and need not travel; capability.json, the engine logs and the report do.
  Get-ChildItem -LiteralPath (Join-Path $data 'check') -Filter 'excerpt-*.wav' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
  $keep = Join-Path $gdir 'capability.json'
  if (Test-Path -LiteralPath (Join-Path $data 'capability.json')) { Copy-Item -LiteralPath (Join-Path $data 'capability.json') -Destination $keep -Force }
  $cellsPath = Join-Path $gdir 'cells.json'
  Write-PackJson -Path $cellsPath -Value @($cells | ForEach-Object { [ordered]@{ pid = $_.Pid; command = $_.Command; started = $_.Started.ToUniversalTime().ToString('o'); seconds = $_.Seconds; model = $_.Model; lang = $_.Lang; device = $_.Device; threads = $_.Threads; mode = $_.Mode; clip = $_.Clip } })

  # Information when the check finished and the recorder safety run passed; a check that did not is INVALID or FAIL.
  $status = $(if ($verdict.Status -eq 'PASS') { 'INFO' } else { $verdict.Status })
  $headline = ($verdict.Headline -replace '^the quick check took', 'the full check took')
  if ($status -eq 'INFO') { $headline = ('{0}; against the quick check: {1} cells compared, {2} flagged above 10%, {3} of them deciding' -f $headline, $cmp.Compared, $cmp.Flagged, $cmp.DecidingFlagged) }
  if ($null -eq $quickCap) { $notes = @('the quick check has not run on this laptop, so the quick-against-full comparison is missing') } else { $notes = @() }
  $evidence = Join-Path $gdir 'full-check.txt'
  $lines = @('Full system check (gate pack)', ("machine: {0}; profile: {1}" -f $ctx.machine, $ctx.profile), ("date: {0}" -f (Get-Date).ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)),
    ("the gate build exited {0} after {1:N1} min" -f $exitCode, $minutes), '') + @($verdict.Lines) + @('', 'quick check against full check on the same rolling cells:') + @($cmp.Lines) + @('', ('FULL-CHECK RESULT: {0}' -f $status))
  foreach ($pr in $verdict.Problems) { $lines += "  problem: $pr" }
  Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel ([string]$ctx.machine)

  $measured = [ordered]@{ check_seconds = $verdict.Numbers.check_seconds; tier = $verdict.Numbers.tier; cells = $verdict.Numbers.cells; compared = $cmp.Compared; flagged = $cmp.Flagged; deciding_flagged = $cmp.DecidingFlagged }
  New-PackResult -Id 'full-check' -Status $status -Headline $headline -Measured $measured -Threshold 'the check completes within the 120 minute limit; recorder safety run passed; quick against full real-time factors reported with the 10% flag' -Evidence @($evidence, $keep, $cellsPath) -Notes $notes -Quick $quick
}
