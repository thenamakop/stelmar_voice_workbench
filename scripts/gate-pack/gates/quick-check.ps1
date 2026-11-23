# Gate quick-check: the app's own quick system check, run through the gate build's WB_GATE_CHECK hook,
# with per-cell times taken from outside (a process watcher on the engine processes), the verdict in
# PowerShell, and capability.json kept. Also answers the owner's 14 Nov 2026 question: the graphics-
# alone run of the hybrid test against the 480 s bar, and whether a graphics adapter slower than 1.5x
# real time on Hindi exhausts the 480 s.
# On a laptop the gate build is started directly; in the dry run on the Legion it is started through
# wb-emulate (which is not in the pack: its path comes from the run's context).
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'GateRun.psm1') -Force
Import-Module (Join-Path $lib 'GateData.psm1') -Force
Import-Module (Join-Path $lib 'Redact.psm1') -Force
Import-Module (Join-Path $lib 'AppProcess.psm1') -Force
Import-Module (Join-Path $lib 'ProcWatch.psm1') -Force
Import-Module (Join-Path $lib 'QuickCheck.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

Invoke-PackGateBody -Ctx $ctx -Id 'quick-check' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $gdir = [string]$ctx.gate_dir
  $run = Join-Path $gdir 'run'
  if (Test-Path -LiteralPath $run) { Remove-Item -LiteralPath $run -Recurse -Force }
  $data = Join-Path $run 'data'
  New-Item -ItemType Directory -Force -Path $data | Out-Null
  $report = Join-Path $run 'report.json'
  foreach ($f in @($paths.GateExe, $paths.BenchClips)) { if (-not (Test-Path -LiteralPath $f)) { throw "the pack is missing $f (run run-gates.ps1 -Verify)" } }
  if ($ctx.profile -ne 'laptop' -and (-not $paths.Emulate -or -not (Test-Path -LiteralPath $paths.Emulate))) { throw "the emulation profile $($ctx.profile) needs wb-emulate; it is not available (dry run on the Legion only)" }

  $env:WB_GATE_CHECK = 'quick'
  $null = Set-WbGateDataDir -Dir $data
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
    Invoke-WbGate -Name 'quick-check' -Body {
      $script:qcWatch = Start-PackProcWatch -Dir (Join-Path $gdir 'watch')
      $started = Get-Date
      if ($ctx.profile -eq 'laptop') {
        $p = Start-Process -FilePath $paths.GateExe -PassThru -WindowStyle Hidden
      } else {
        $a = @('--profile', ([string]$ctx.profile), '--log', (ConvertTo-PackArg (Join-Path $run 'emulate.log')), '--', (ConvertTo-PackArg $paths.GateExe))
        $p = Start-Process -FilePath $paths.Emulate -ArgumentList $a -PassThru -WindowStyle Hidden
      }
      $null = $p.Handle
      $deadline = $started.AddMinutes(25)
      while (-not $p.HasExited) {
        if ((Get-Date) -gt $deadline) {
          [void](Stop-WbApp)
          Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
          throw 'the quick check did not finish within 25 minutes; stopped it'
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
  $verdict = Get-QuickCheckVerdict -Report $rep -Capability $cap -Cells $cells
  # The excerpt clips are large and need not travel; capability.json, the engine logs and the report do.
  Get-ChildItem -LiteralPath (Join-Path $data 'check') -Filter 'excerpt-*.wav' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
  $keep = Join-Path $gdir 'capability.json'
  if (Test-Path -LiteralPath (Join-Path $data 'capability.json')) { Copy-Item -LiteralPath (Join-Path $data 'capability.json') -Destination $keep -Force }
  $cellsPath = Join-Path $gdir 'cells.json'
  Write-PackJson -Path $cellsPath -Value @($cells | ForEach-Object { [ordered]@{ pid = $_.Pid; command = $_.Command; started = $_.Started.ToUniversalTime().ToString('o'); seconds = $_.Seconds; model = $_.Model; lang = $_.Lang; device = $_.Device; threads = $_.Threads; mode = $_.Mode; clip = $_.Clip } })

  $evidence = Join-Path $gdir 'quick-check.txt'
  $lines = @('Quick system check (gate pack)', ("machine: {0}; profile: {1}" -f $ctx.machine, $ctx.profile), ("date: {0}" -f (Get-Date).ToString('yyyy-MM-dd HH:mm', [Globalization.CultureInfo]::InvariantCulture)),
    ("the gate build exited {0} after {1:N1} min" -f $exitCode, $minutes), '') + @($verdict.Lines) + @('', ('QUICK-CHECK RESULT: {0}' -f $verdict.Status))
  foreach ($pr in $verdict.Problems) { $lines += "  problem: $pr" }
  Write-WbEvidence -Path $evidence -Lines $lines -MachineLabel ([string]$ctx.machine)

  New-PackResult -Id 'quick-check' -Status $verdict.Status -Headline $verdict.Headline -Measured $verdict.Numbers -Threshold 'quick check ends within 480 s; recorder safety run passed; no app error; capability.json kept' -Evidence @($evidence, $keep, $cellsPath) -Quick $quick
}
