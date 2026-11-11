<#
.SYNOPSIS
  Phase 2 renamed-engines test: with every engine renamed away, the app still records and shows a clear message.

.DESCRIPTION
  Two short recorder runs of the gate build under an emulation profile (default emu-low), each through
  recorder-gate.ps1 -Gate soak -Minutes <n> (virtual cable, known noise file, the full recorder checks):
    1. engines renamed away: a scratch folder holds copies of the four engines under names the app does not
       recognise (wb-engine-<variant>-x86_64-pc-windows-msvc.exe.renamed); the app is pointed at it with
       WB_ENGINES_DIR. Expected: the recorder run passes its own checks (duration, no gap, zero overruns) and
       the app log holds the warning "No speech engine found in ... Recording is not affected ..." (the notice
       goes to the log and, if the UI is listening, to a Toast); the Record screen shows a persistent banner read from
       the app state (see step 3 below);
    2. control, engines intact: the same run with WB_ENGINES_DIR pointing at the real engines: the app log must
       NOT hold that message, so the message in run 1 is caused by the renaming.
  Takes the gate lock through recorder-gate.ps1 (timing-sensitive rules apply); about 12 minutes in all.
  Evidence: docs\evidence\phase-2\rename-engines-<profile>.txt (the two recorder-gate files are kept beside it).
#>
[CmdletBinding()]
param(
  [ValidateSet('emu-low', 'emu-mid', 'native')] [string]$Profile = 'emu-low',
  [double]$Minutes = 3,
  [string]$Machine = 'Legion',
  [string]$Scratch = 'D:\STELMAR-Gate\rename-engines',
  [string]$OutDir,
  [switch]$AllowCableDefault
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-2' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
$inv = [Globalization.CultureInfo]::InvariantCulture
$triple = 'x86_64-pc-windows-msvc'
$logFile = Join-Path $env:LOCALAPPDATA 'in.edu.bmu.stelmar.voiceworkbench\logs\app.log'
$gateScript = Join-Path $here 'recorder-gate.ps1'
$msgPattern = 'No speech engine found'

$renamed = Join-Path $Scratch 'renamed'
$intact = Join-Path $Scratch 'intact'
foreach ($d in @($renamed, $intact)) { if (Test-Path $d) { Remove-Item $d -Recurse -Force }; New-Item -ItemType Directory -Force -Path $d | Out-Null }
$names = @()
foreach ($v in @('avx2', 'avx512', 'vulkan', 'cuda')) {
  $src = Join-Path $repo "src-tauri\binaries\wb-engine-$v-$triple.exe"
  if (-not (Test-Path $src)) { continue }
  # Small stand-ins would be enough for "renamed", but real copies keep the test honest about file sizes.
  Copy-Item $src (Join-Path $renamed "wb-engine-$v-$triple.exe.renamed")
  Copy-Item $src (Join-Path $intact "wb-engine-$v-$triple.exe")
  $names += $v
}
if (-not $names.Count) { throw 'no engine built: nothing to rename (scripts\build-engines.ps1 first)' }

function Invoke-Run([string]$Label, [string]$EnginesDir) {
  $before = $(if (Test-Path $logFile) { (Get-Item $logFile).Length } else { 0 })
  $out = Join-Path $Scratch "recorder-$Label"
  New-Item -ItemType Directory -Force -Path $out | Out-Null
  $env:WB_ENGINES_DIR = $EnginesDir
  $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $gateScript, '-Gate', 'soak', '-Profile', $Profile, '-Minutes', $Minutes.ToString($inv), '-OutDir', $out)
  if ($AllowCableDefault) { $argList += '-AllowCableDefault' }
  try {
    $r = Invoke-WbNative -FilePath 'powershell.exe' -ArgumentList $argList
  } finally { Remove-Item Env:\WB_ENGINES_DIR -ErrorAction SilentlyContinue }
  $logLines = @()
  if (Test-Path $logFile) {
    $fs = [IO.File]::Open($logFile, 'Open', 'Read', 'ReadWrite')
    try { [void]$fs.Seek([math]::Min($before, $fs.Length), 'Begin'); $sr = New-Object IO.StreamReader($fs); $logLines = @($sr.ReadToEnd() -split "`r?`n" | Where-Object { $_ }) } finally { $fs.Dispose() }
  }
  $hits = @($logLines | Where-Object { $_ -match $msgPattern })
  $evFile = Join-Path $out "soak-$Profile.txt"
  [pscustomobject]@{ Label = $Label; Exit = $r.ExitCode; Hits = $hits; Evidence = $evFile; EvidenceLines = $(if (Test-Path $evFile) { @(Get-Content $evFile) } else { @() }); Console = @($r.Output | Select-Object -Last 6) }
}

$run1 = Invoke-Run 'renamed' $renamed
$run2 = Invoke-Run 'intact' $intact

# 3. What the operator sees: the app is started with the engines renamed away, then with them present (control).
# Each time the Record screen is opened through the window's accessibility tree (no mouse or keyboard input), the
# banner text is looked for there, and the window is captured (window bounds only, only while it is in front; never
# the desktop) at about 1, 3 and 5 s after the screen opened. The banner is "No speech engine found. Recording is
# not affected." and must show with the engines renamed away and not with them present.
Import-Module (Join-Path $repo 'scripts\lib\AppProcess.psm1') -Force
$shotDir = Join-Path $OutDir 'ui'
New-Item -ItemType Directory -Force -Path $shotDir | Out-Null
$bannerText = 'No speech engine found.'
function Invoke-UiRun([string]$Tag, [string]$EnginesDir, [int[]]$Waits) {
  $r = [pscustomobject]@{ Tag = $Tag; Opened = $false; BannerFound = $false; Shots = @(); Note = '' }
  $env:WB_ENGINES_DIR = $EnginesDir
  try {
    if (Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue) { throw 'the app is already running' }
    $app = Start-WbApp -Exe (Join-Path $repo 'target\gate\stelmar-voice-workbench.exe') -TimeoutSeconds 30
    $r.Opened = Invoke-WbAppButton -Handle $app.Handle -Name 'Record'
    if (-not $r.Opened) { throw 'the Record button was not found in the window' }
    # Present: found at once. Absent: the whole wait (15 s, longer than the engine probe) passes without it.
    $r.BannerFound = Test-WbAppText -Handle $app.Handle -Text $bannerText -TimeoutSeconds 15
    $n = 0
    foreach ($sec in $Waits) {
      Start-Sleep -Seconds $sec
      $n++
      $png = Join-Path $shotDir "rename-engines-$Profile-$Tag-shot$n.png"
      $c = Get-WbWindowCheck -Handle $app.Handle -SavePath $png
      $r.Shots += [pscustomobject]@{ Path = $png; Rendered = [bool]$c.Rendered; Obscured = [bool]$c.Obscured; Seconds = [math]::Round($app.Watch.Elapsed.TotalSeconds, 0) }
    }
  } catch { $r.Note = "not completed: $($_.Exception.Message)" }
  finally { Remove-Item Env:\WB_ENGINES_DIR -ErrorAction SilentlyContinue; try { $null = Stop-WbApp } catch { } }
  $r
}
# Old captures from earlier versions of this script are replaced.
Get-ChildItem -Path $shotDir -Filter "rename-engines-$Profile-*shot*.png" -ErrorAction SilentlyContinue | Remove-Item -Force
# Whatever else the function wrote to the pipeline, the result is its last object.
$ui1 = @(Invoke-UiRun 'renamed' $renamed @(1, 2, 2))[-1]
$ui2 = @(Invoke-UiRun 'intact' $intact @(1))[-1]

function Get-Verdict($run) { ($run.EvidenceLines | Where-Object { $_ -match '^verdict:' } | Select-Object -First 1) }
$ok1 = ($run1.Exit -eq 0) -and ($run1.Hits.Count -ge 1)
$ok2 = ($run2.Exit -eq 0) -and ($run2.Hits.Count -eq 0)
$ok3 = $ui1.Opened -and $ui1.BannerFound -and ($ui1.Shots.Count -ge 1) -and (@($ui1.Shots | Where-Object { $_.Obscured }).Count -eq 0)
$ok4 = $ui2.Opened -and (-not $ui2.BannerFound) -and ($ui2.Shots.Count -ge 1) -and (@($ui2.Shots | Where-Object { $_.Obscured }).Count -eq 0)
$pass = $ok1 -and $ok2 -and $ok3 -and $ok4
$lines = @(
  "Renamed-engines test (Phase 2), profile $Profile",
  "date: $((Get-Date).ToString('yyyy-MM-dd HH:mm', $inv)) (IST on the Legion); machine: $Machine; gate build target\gate\stelmar-voice-workbench.exe; recorder run $Minutes min each through recorder-gate.ps1 -Gate soak -Minutes $Minutes",
  "engines renamed: $($names -join ', ') (copies named wb-engine-<variant>-$triple.exe.renamed in a scratch folder; the app is pointed at it with WB_ENGINES_DIR)",
  '',
  '1. engines renamed away',
  "   recorder run: exit code $($run1.Exit); $(Get-Verdict $run1)",
  "   app log lines holding the message ($($run1.Hits.Count)):"
)
foreach ($h in ($run1.Hits | Select-Object -First 3)) { $lines += "     $h" }
$lines += $(if ($ok1) { '   => the recording ran and passed its own checks, and the app said so: ok' } else { '   => WRONG: the run did not pass, or the message is missing' })
$lines += ''
$lines += '2. control, engines intact (same run, WB_ENGINES_DIR = the intact copies)'
$lines += "   recorder run: exit code $($run2.Exit); $(Get-Verdict $run2)"
$lines += "   app log lines holding the message: $($run2.Hits.Count) (expected 0)"
$lines += $(if ($ok2) { '   => ok: without the renaming there is no such message' } else { '   => WRONG' })
$lines += ''
$lines += ''
$lines += '3. what the operator sees: the Record screen (opened through the window''s accessibility tree; windows captured: bounds only, foreground-checked, never the desktop)'
foreach ($u in @($ui1, $ui2)) {
  $what = $(if ($u.Tag -eq 'renamed') { 'engines renamed away' } else { 'control, engines present' })
  $lines += "   $what : Record opened $($u.Opened); text '$bannerText' found in the window: $($u.BannerFound)$(if ($u.Note) { '; ' + $u.Note })"
  foreach ($sh in $u.Shots) { $lines += "     $(Split-Path $sh.Path -Leaf), $($sh.Seconds) s after launch: rendered $($sh.Rendered), obscured $($sh.Obscured)" }
}
$lines += $(if ($ok3 -and $ok4) { '   => the banner shows with the engines renamed away and not with them present: ok' } else { '   => WRONG: the banner is missing with the engines renamed away, or shows with them present' })
$lines += '   Read from the PNGs (by the agent, not by the script): see the note at the end of this file.'
$lines += ''
$lines += "the recorder evidence of both runs: $($run1.Evidence) and $($run2.Evidence) (scratch folder, not committed; audio never leaves D:\STELMAR-Gate)"
if (-not $ok1) { $lines += ($run1.Console | ForEach-Object { "   console: $_" }) }
if (-not $ok2) { $lines += ($run2.Console | ForEach-Object { "   console: $_" }) }
$lines += $(if ($pass) { "verdict: PASS$(if ($Profile -ne 'native') { ' (PROVISIONAL: emulated)' })" } else { 'verdict: FAIL' })
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$path = Join-Path $OutDir "rename-engines-$Profile.txt"
Write-WbEvidence -Path $path -Lines $lines -MachineLabel $Machine
Write-Host "evidence: $path"
$lines | Select-Object -Last 4
if (-not $pass) { exit 1 }
