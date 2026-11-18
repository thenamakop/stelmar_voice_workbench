<#
.SYNOPSIS
  A short native -Quick smoke of the gate pack on the Legion: -Profile laptop, no wb-emulate.

.DESCRIPTION
  The dry run on the Legion goes through wb-emulate and VB-CABLE, so it never takes the code paths a laptop
  takes. This runs the BUILT pack (run build-gate-pack.ps1 first) the way a laptop would: the profile
  'laptop', logical processors from the machine, the integrated graphics adapter chosen from
  wb-engine-vulkan info, the rig found by Resolve-PackRig (it falls back to VB-CABLE here, loudly), and
  the gate lock taken in the folder given with -LockDir.

  It is NOT a laptop result. It never writes a results zip (-NoZip) and never calls the ingest script, so
  nothing from it can reach docs\GATES.md. It writes SMOKE-REPORT.txt in its own folder: each gate's status
  and headline, which engine device the beside-recorder gates picked, the rig kind, and a check that
  'wb-emulate' appears in no log.

  Left out by default: the microphone run (it plays speech aloud: -IncludeMic, only when a person is
  there), rename-engines (needs an unlocked desktop: -IncludeRename), the network block (needs the Wi-Fi
  switch), the settled load (needs 3 hours of uptime), the gates that need a restart, and the supervisor
  kill. Takes the gate lock: run it only when nothing else holds it.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\gate-pack-native-smoke.ps1 -PackRoot D:\STELMAR-Gate\pack
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$PackRoot,
  [string]$Root = 'D:\STELMAR-Gate\native-smoke',
  [string]$LockDir = 'D:\Projects',
  [switch]$IncludeMic,
  [switch]$IncludeRename,
  [string[]]$ExtraSkip = @(),
  [int]$TimeoutMinutes = 360
)
$ErrorActionPreference = 'Stop'
$PackRoot = (Resolve-Path $PackRoot).Path
$runner = Join-Path $PackRoot 'run-gates.ps1'
if (-not (Test-Path -LiteralPath $runner)) { throw "no run-gates.ps1 in $PackRoot (build the pack first)" }
$work = Join-Path $Root 'work'
$backup = Join-Path $Root 'backup'
New-Item -ItemType Directory -Force -Path $work, $backup | Out-Null
$skip = New-Object System.Collections.Generic.List[string]
foreach ($s in 'network-off', 'load-settled', 'load-cold', 'hashing-1', 'supervisor-kill') { $skip.Add($s) }
if (-not $IncludeMic) { $skip.Add('mic-run') }
if (-not $IncludeRename) { $skip.Add('rename-engines') }
foreach ($s in $ExtraSkip) { $skip.Add($s) }

$args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f $runner), '-Profile', 'laptop', '-Role', 'low', '-Quick', '-NoRestart', '-NoZip', '-Unattended', '-UnattendedWaitMinutes', '1',
  '-PackRoot', ('"{0}"' -f $PackRoot), '-WorkDir', ('"{0}"' -f $work), '-LockDir', ('"{0}"' -f $LockDir), '-BackupRoot', ('"{0}"' -f $backup), '-RunLabel', 'native-smoke-legion',
  '-Skip', ($skip -join ','))
# -IncludeMic is a person at the machine: the runner runs the gate attended, not unattended.
if ($IncludeMic -or $IncludeRename) { $args = @($args | Where-Object { $_ -notin '-Unattended', '-UnattendedWaitMinutes', '1' }) }
Write-Host ('native smoke: {0} -Quick, skipping {1}' -f $PackRoot, ($skip -join ', '))
$p = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -ArgumentList $args -PassThru -Wait:$false -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Root 'smoke.out.log') -RedirectStandardError (Join-Path $Root 'smoke.err.log')
if (-not $p.WaitForExit($TimeoutMinutes * 60 * 1000)) { Stop-Process -Id $p.Id -Force; Write-Host 'the smoke run did not finish in time and was stopped' -ForegroundColor Red }

$summaryPath = Join-Path $work 'summary.json'
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('Native -Quick smoke of the gate pack on the Legion (profile laptop, no wb-emulate). NOT a laptop result; never ingested.')
$lines.Add(('date: {0}; pack: {1}; runner exit code: {2}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm'), $PackRoot, $(if ($p.HasExited) { $p.ExitCode } else { 'stopped' })))
if (Test-Path -LiteralPath $summaryPath) {
  $sum = Get-Content -LiteralPath $summaryPath -Raw -Encoding UTF8 | ConvertFrom-Json
  foreach ($g in @($sum.gates)) { $lines.Add(('{0,-8} {1,-20} {2}' -f $g.status, $g.id, $g.headline)) }
  foreach ($w in @($sum.pending)) { $lines.Add(('WAITING  {0,-20} {1}' -f $w.id, $w.reason)) }
  foreach ($n in @($sum.not_in_pack)) { $lines.Add(('NOT IN PACK {0}' -f $n.id)) }
} else { $lines.Add('no summary.json was written: see smoke.out.log and smoke.err.log') }
# What the laptop-mode paths did.
$rigs = @(Get-ChildItem -LiteralPath (Join-Path $work 'gates') -Recurse -Filter 'result.json' -ErrorAction SilentlyContinue | ForEach-Object { try { (Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json) } catch { $null } } | Where-Object { $_ -and $_.rig } | ForEach-Object { '{0}: {1}' -f $_.id, $_.rig })
$lines.Add('rig used per gate: ' + $(if ($rigs.Count -gt 0) { $rigs -join '; ' } else { 'none recorded' }))
$beside = @(Get-ChildItem -LiteralPath (Join-Path $work 'gates') -Recurse -Filter 'result.json' -ErrorAction SilentlyContinue | ForEach-Object { try { (Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json) } catch { $null } } | Where-Object { $_ -and $_.measured -and $_.measured.PSObject.Properties['beside'] } | ForEach-Object { '{0}: {1}' -f $_.id, $_.measured.beside.batch })
$lines.Add('batch device picked beside the recorder: ' + $(if ($beside.Count -gt 0) { $beside -join '; ' } else { 'not run' }))
$emuHits = @(Get-ChildItem -LiteralPath $work -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Extension -in '.log', '.txt', '.json' } | Select-String -Pattern 'wb-emulate' -List | ForEach-Object { $_.Path })
$lines.Add('wb-emulate mentioned in the logs: ' + $(if ($emuHits.Count -eq 0) { 'no (good)' } else { 'YES in ' + (($emuHits | Select-Object -First 5) -join ', ') }))
$report = Join-Path $Root 'SMOKE-REPORT.txt'
Set-Content -LiteralPath $report -Value @($lines) -Encoding UTF8
$lines | ForEach-Object { Write-Host $_ }
Write-Host "report: $report"
