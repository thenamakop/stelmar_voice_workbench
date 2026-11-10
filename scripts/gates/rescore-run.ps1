<#
.SYNOPSIS
  Scores a finished gate run again under the recording-gate rule of 9 Nov 2026 (docs/PLAN.md
  section 12) and writes only the rule's part of the evidence: fidelity, upstream breaks and
  validity. For gates that have no -FromRun (the force-kill test), whose other checks need data
  that was not saved.

.DESCRIPTION
  Reads the run folder's saved recording, reference capture, analysis files and reports, runs
  `wb-testbench compare`, and writes <OutDir>\<name>.txt. The original run's own evidence stands
  for every other check of that gate; this file says so.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$Run,
  [Parameter(Mandatory = $true)] [string]$Name,
  [string]$OutDir,
  [string]$Testbench,
  [string]$Machine = 'Legion',
  [string]$Note = '',
  # The force-kill test stops the app before the player (see Invoke-WbCompare).
  [switch]$IgnoreReferenceTail
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Fidelity.psm1') -Force
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\release\wb-testbench.exe' }
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-1\rescored' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

function Read-Json([string]$Path) { if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json } else { $null } }
$sessionDir = (Get-ChildItem (Join-Path $Run 'rec') -Directory | ForEach-Object { Get-ChildItem $_.FullName -Directory } | Select-Object -First 1).FullName
$refDir = Join-Path $Run 'reference'
$cmp = Invoke-WbCompare -Testbench $Testbench -SessionDir $sessionDir -ReferenceDir $refDir -RunDir $Run -IgnoreReferenceTail:$IgnoreReferenceTail
$compare = Read-Json (Join-Path $Run 'compare.json')
$play = Read-Json (Join-Path $Run 'play.json')
$refAnalysis = Read-Json (Join-Path $Run 'reference-analysis.json')
$reference = Read-Json (Join-Path $Run 'reference.json')
$report = Read-Json (Join-Path $Run 'stop-report.json')
$dropped = 0
if ($report -and (Test-WbHasProperty $report 'stopped') -and $report.stopped) { $dropped = [int64]$report.stopped.recorder.dropped_samples }
$playStart = $(if ($play -and (Test-WbHasProperty $play 'started_at')) { [DateTimeOffset]::Parse($play.started_at) } else { $null })
$rv = Get-WbRunVerdict -Compare $compare -RefAnalysis $refAnalysis -Play $play -Reference $reference -PlayStarted $playStart -DroppedSamples $dropped

$lines = @()
$lines += "re-scored: $Name under the recording-gate rule of 9 Nov 2026 (docs/PLAN.md section 12)"
$lines += "run folder: $Run"
$lines += "build: stelmar-voice-workbench.exe sha256 1bbf39e5884849ffab144de3026e25ee5519c37266e6dd55c0d4a6c369a698d0 (final gate build); wb-testbench.exe sha256 $((Get-FileHash -LiteralPath $Testbench -Algorithm SHA256).Hash.ToLowerInvariant())"
$lines += "note: only the rule's part is re-scored (fidelity, upstream breaks, validity). The other checks of this gate (loss at each kill, parts playable before repair, Resume offered, parts.json, mirror, xruns) need data the original run did not save; its own evidence stands for them. $Note"
$lines += "verdict under the rule: $(if ($rv.Verdict -eq 'INVALID') { 'INVALID (no verdict; the run must be repeated)' } else { $rv.Verdict })"
$lines += ''
$lines += Get-WbVerdictLines -Result $rv -Compare $compare
if ($cmp.ExitCode -ne 0) { $lines += "comparison error: $((@($cmp.Error) + @($cmp.Output)) -join ' ')" }
$path = Join-Path $OutDir "$Name.txt"
Write-WbEvidence -Path $path -Lines $lines -MachineLabel $Machine
Write-Host "$($rv.Verdict): $path"
if ($rv.Verdict -eq 'INVALID') { exit 3 }
if ($rv.Verdict -eq 'FAIL') { exit 1 }
