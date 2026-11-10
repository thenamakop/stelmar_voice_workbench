<#
.SYNOPSIS
  Scores the live detector check of a finished gate run again from its saved data (no recording, no timing):
  the shared upstream breaks the harness found against the repeated_block and zero_gap events in the
  run's session.json, with the lead-in class of 11 Nov 2026. Prints the evidence lines.

.DESCRIPTION
  Reads <Run>\analysis.json, reference-analysis.json, play.json and the session.json under <Run>\rec, the same
  inputs recorder-gate.ps1 and kill-gate.ps1 used, and calls Get-WbDetectorMatches / Get-WbDetectorLines.
  It is how the soak of Night 1 (11 Nov 2026) was re-scored after the lead-in class was added.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$Run
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Fidelity.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Detector.psm1') -Force

function Read-Json([string]$Path) { if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json } else { $null } }
$sess = @(Get-ChildItem (Join-Path $Run 'rec') -Directory | ForEach-Object { Get-ChildItem $_.FullName -Directory -Filter 'gate-*' } | Select-Object -First 1)
if ($sess.Count -eq 0) { throw "no session under $Run\rec" }
$analysis = Read-Json (Join-Path $Run 'analysis.json')
$refAnalysis = Read-Json (Join-Path $Run 'reference-analysis.json')
$play = Read-Json (Join-Path $Run 'play.json')
$playStart = $(if ($play -and (Test-WbHasProperty $play 'started_at')) { [DateTimeOffset]::Parse($play.started_at) } else { $null })
if (-not ($analysis -and $refAnalysis)) { throw 'the run has no analysis.json or reference-analysis.json' }
$meta = Get-Content -LiteralPath (Join-Path $sess[0].FullName 'session.json') -Raw | ConvertFrom-Json
$breaks = @(Get-WbUpstreamBreaks -RefAnalysis $refAnalysis -Play $play -PlayStarted $playStart)
$check = Get-WbDetectorMatches -Breaks $breaks -Analysis $analysis -Events @($meta.events)
Get-WbDetectorLines -Check $check -Rec $null
