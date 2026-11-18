<#
  RecorderGates.psm1: the pack's wrapper around scripts\gates\recorder-gate.ps1 (soak, load, panic,
  webview). It starts the gate in laptop mode (or in the emulated profile of the dry run), with the
  hardware-interface rig and the second WASAPI reference, samples the processor while it runs, and then
  adds what the old gate does not say: the thermal summary, which rig was used, and how often the
  zero_gap and repeated_block detectors fired against what the two references show.

  Pure helpers (outcome, run folder, detector matching) are tested. Windows PowerShell 5.1, ASCII only.
#>

Set-StrictMode -Version 2

Import-Module (Join-Path $PSScriptRoot 'PackCore.psm1')
Import-Module (Join-Path $PSScriptRoot 'PackGate.psm1')
Import-Module (Join-Path $PSScriptRoot 'PackNative.psm1')
Import-Module (Join-Path $PSScriptRoot 'RefVerdict.psm1')
Import-Module (Join-Path $PSScriptRoot 'LoopbackRig.psm1')
Import-Module (Join-Path $PSScriptRoot 'Thermal.psm1')

# Pure. recorder-gate exits 0 PASS, 1 FAIL, 3 INVALID; anything else is a fault in the run.
function Get-PackRecorderOutcome {
  param([int]$ExitCode, [string[]]$EvidenceLines = @())
  $verdictLine = @($EvidenceLines | Where-Object { $_ -match '^verdict:' } | Select-Object -First 1)
  switch ($ExitCode) {
    0 { return [pscustomobject]@{ Status = 'PASS'; VerdictLine = [string]($verdictLine -join '') } }
    1 { return [pscustomobject]@{ Status = 'FAIL'; VerdictLine = [string]($verdictLine -join '') } }
    default { return [pscustomobject]@{ Status = 'INVALID'; VerdictLine = $(if ($verdictLine) { [string]($verdictLine -join '') } else { "the gate ended with exit code $ExitCode" }) } }
  }
}

# Pure. The checks of the evidence that did not hold (the lines "  [ ] ..."), so a FAIL says why in the headline
# and not just "FAIL".
function Get-PackFailedChecks {
  param([string[]]$EvidenceLines = @())
  @($EvidenceLines | Where-Object { $_ -match '^\s*\[ \]\s+\S' } | ForEach-Object { ($_ -replace '^\s*\[ \]\s+', '').Trim() })
}

# Pure. The run folder named in the evidence ("audio and reports: <path> (not in the repository)").
function Get-PackRunDirFromEvidence {
  param([string[]]$EvidenceLines = @())
  foreach ($l in $EvidenceLines) { if ($l -match '^audio and reports:\s+(.+?)\s+\(not in the repository\)') { return $Matches[1] } }
  ''
}

# Pure. Which rig the evidence says was used.
function Get-PackRigKindFromEvidence {
  param([string[]]$EvidenceLines = @())
  $t = $EvidenceLines -join "`n"
  if ($t -match 'FALLBACK RIG: VB-CABLE') { return 'vb-cable-fallback' }
  if ($t -match 'HARDWARE LOOPBACK on the audio interface') { return 'loopback' }
  'vb-cable'
}

# Pure. The offset (ms) that maps a time on a reference's clock to the app's clock: the first aligned
# segment of a compare.json. $null when there is none.
function Get-PackCompareOffsetMs {
  param($Compare, [double]$Rate = 48000)
  if ($null -eq $Compare) { return $null }
  $segP = $Compare.PSObject.Properties['segments']
  if ($null -eq $segP) { return $null }
  $seg = @($segP.Value | Where-Object { $_.PSObject.Properties['offset_frames'] -and $null -ne $_.offset_frames -and ($_.PSObject.Properties['aligned'] -eq $null -or [bool]$_.aligned) } | Select-Object -First 1)
  if ($seg.Count -eq 0) { return $null }
  1000.0 * [double]$seg[0].offset_frames / $Rate
}

# Pure. Moves detector findings from a reference's clock to the app's: each finding gets the offset of the
# aligned segment of compare.json that covers it (a long run has a new offset after every unmatched stretch);
# one outside every segment gets the first segment's offset (the app has nothing there to match).
function Get-PackAppTimeFindings {
  param($Findings, $Compare, [double]$Rate = 48000)
  $first = Get-PackCompareOffsetMs -Compare $Compare -Rate $Rate
  $firstMs = $(if ($null -ne $first) { [double]$first } else { 0.0 })
  $segs = @()
  if ($null -ne $Compare -and $null -ne $Compare.PSObject.Properties['segments']) {
    $segs = @($Compare.segments | Where-Object { $_.PSObject.Properties['offset_frames'] -and $null -ne $_.offset_frames -and $_.PSObject.Properties['reference_end_frame'] -and ($_.PSObject.Properties['aligned'] -eq $null -or [bool]$_.aligned) })
  }
  @($Findings | ForEach-Object {
      $t = [double]$_.AudioMs
      $fr = $t * $Rate / 1000.0
      $s = @($segs | Where-Object { $fr -ge [double]$_.reference_start_frame -and $fr -lt [double]$_.reference_end_frame } | Select-Object -First 1)
      $off = $(if ($s.Count -gt 0) { 1000.0 * [double]$s[0].offset_frames / $Rate } else { $firstMs })
      [pscustomobject]@{ Kind = $_.Kind; AudioMs = $t + $off }
    })
}

# Pure. Detector findings (Kind, AudioMs) from a scan-repeats report.
function Get-PackScanFindings {
  param($Scan, [string]$Kind)
  if ($null -eq $Scan) { return @() }
  $src = $(if ($Kind -eq 'repeated_block') { 'blocks' } else { 'zero_gaps' })
  $p = $Scan.PSObject.Properties[$src]
  if ($null -eq $p) { return @() }
  @($p.Value | ForEach-Object { [pscustomobject]@{ Kind = $Kind; AudioMs = [double]$_.audio_ms } })
}

# Detector summary of one run: firings from session.log against both references. Returns Lines and Rates.
function Get-PackRunDetectors {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] [string]$RunDir, [Parameter(Mandatory = $true)] [string]$Testbench, [string]$Conditions = '',
    # Scripted windows (StartMs, EndMs on the source clock) and the app clock minus the source clock: firings and
    # reference findings inside them are left out of the rates (the digital zeros of a mute are not a dropout) and counted apart.
    $ExcludeWindows = @(), [double]$WindowOffsetMs = 0
  )
  $lines = New-Object System.Collections.Generic.List[string]
  $rates = [ordered]@{}
  $log = @(Get-ChildItem -LiteralPath (Join-Path $RunDir 'rec') -Recurse -Filter 'session.log' -ErrorAction SilentlyContinue | Select-Object -First 1)
  if ($log.Count -eq 0) { return [pscustomobject]@{ Lines = @('detector rates: no session.log in the run folder'); Rates = $rates } }
  $firings = @(ConvertFrom-PackSessionLog -Lines @(Get-Content -LiteralPath $log[0].FullName -Encoding UTF8))
  $sessionDir = Split-Path -Parent $log[0].FullName
  $scan = { param($dir)
    if (-not (Test-Path -LiteralPath $dir)) { return $null }
    $out = Join-Path $RunDir ('scan-' + (Split-Path -Leaf $dir) + '.json')
    $r = Invoke-PackNative -FilePath $Testbench -ArgumentList @('scan-repeats', '--session', $dir, '--report', $out) -TimeoutSeconds 1800
    if ($r.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $out)) { return $null }
    Get-Content -LiteralPath $out -Raw -Encoding UTF8 | ConvertFrom-Json
  }
  $cp = & $scan (Join-Path $RunDir 'reference')
  $wa = & $scan (Join-Path $RunDir 'reference2')
  $appScan = $null
  $audioMs = 0.0
  $rate = 48000.0
  if ($cp -and $cp.PSObject.Properties['sample_rate']) { $rate = [double]$cp.sample_rate }
  $metaFile = Join-Path $sessionDir 'session.json'
  if (Test-Path -LiteralPath $metaFile) {
    $m = Get-Content -LiteralPath $metaFile -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($n in 'audio_ms', 'total_audio_ms') { if ($m.PSObject.Properties[$n]) { $audioMs = [double]$m.$n; break } }
  }
  if ($audioMs -le 0 -and $cp -and $cp.PSObject.Properties['audio_seconds']) { $audioMs = 1000.0 * [double]$cp.audio_seconds }
  $cmpA = Read-PackJson -Path (Join-Path $RunDir 'compare.json')
  $cmpW = Read-PackJson -Path (Join-Path $RunDir 'compare2.json')
  $offA = Get-PackCompareOffsetMs -Compare $cmpA -Rate $rate
  $offW = Get-PackCompareOffsetMs -Compare $cmpW -Rate $rate
  $inWin = { param($t) @($ExcludeWindows | Where-Object { $t -ge ([double]$_.StartMs + $WindowOffsetMs) -and $t -le ([double]$_.EndMs + $WindowOffsetMs) }).Count -gt 0 }
  foreach ($kind in 'zero_gap', 'repeated_block') {
    $f = @($firings | Where-Object { $_.Kind -eq $kind })
    # App time = reference time + the offset of the segment of compare.json that covers it (app frame = reference
    # frame + offset_frames); an unknown offset is treated as 0 and said so below.
    $c = @(Get-PackAppTimeFindings -Findings @(Get-PackScanFindings -Scan $cp -Kind $kind) -Compare $cmpA -Rate $rate)
    $w = @(Get-PackAppTimeFindings -Findings @(Get-PackScanFindings -Scan $wa -Kind $kind) -Compare $cmpW -Rate $rate)
    $fWin = @($f | Where-Object { & $inWin ([double]$_.AudioMs) })
    if ($ExcludeWindows.Count -gt 0) {
      $f = @($f | Where-Object { -not (& $inWin ([double]$_.AudioMs)) })
      $c = @($c | Where-Object { -not (& $inWin ([double]$_.AudioMs)) })
      $w = @($w | Where-Object { -not (& $inWin ([double]$_.AudioMs)) })
    }
    $ra = Get-PackDetectorRates -Firings $f -CpalFindings $c -WasapiFindings $w -AudioMs $audioMs
    $rates[$kind] = $ra
    if ($ExcludeWindows.Count -gt 0) { $lines.Add(('  detector {0}: {1} firing(s) inside the scripted windows are left out of the rates (see the per-window lines)' -f $kind, $fWin.Count)) }
    $lines.Add(('  detector ' + (Get-PackDetectorLine -Kind $kind -Rates $ra -Conditions $Conditions)))
  }
  if ($null -eq $offA -or $null -eq $offW) { $lines.Add('  detector note: a reference offset was not available, so firings are matched without it (times may not line up)') }
  if (-not $cp) { $lines.Add('  detector note: the cpal reference could not be scanned') }
  if (-not $wa) { $lines.Add('  detector note: the WASAPI reference could not be scanned') }
  [pscustomobject]@{ Lines = @($lines.ToArray()); Rates = $rates }
}

# Runs one recorder gate of the pack. -Gate is soak, load, panic or webview.
function Invoke-PackRecorderGate {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true)] $Ctx,
    [Parameter(Mandatory = $true)] [string]$Id,
    [Parameter(Mandatory = $true)] [ValidateSet('soak', 'load', 'panic', 'webview', 'kill', 'hashing')] [string]$Gate,
    [double]$FullMinutes = 0,
    [double]$QuickMinutes = 0,
    [int]$FullKills = 5,
    [int]$QuickKills = 2,
    [switch]$Thermal,
    [string]$Threshold = '',
    # Engines beside the recorder: a hashtable merged into the config of scripts\lib\beside-load.ps1
    # (mode, batch_device, batch_variant, ...). $null = nothing beside the recorder.
    [hashtable]$Beside = $null,
    # Keeps the evidence of a repeated gate apart: hashing-<profile>-<tag>.txt, <id>-<tag>-additions.txt.
    [string]$RunTag = ''
  )
  $quick = [bool]$Ctx.quick
  $gdir = [string]$Ctx.gate_dir
  $paths = Get-PackPaths -Ctx $Ctx
  foreach ($f in @($paths.GateExe, $paths.Testbench)) { if (-not (Test-Path -LiteralPath $f)) { throw "the pack is missing $f (run run-gates.ps1 -Verify)" } }
  $minutes = $(if ($quick -and $QuickMinutes -gt 0) { $QuickMinutes } else { $FullMinutes })
  Assert-PackFreeDisk -Path ([string]$Ctx.work_dir) -NeedGB (Get-PackDiskNeedGB -Minutes $(if ($Gate -eq 'kill') { 20 } else { $minutes })) -What "the $Gate recordings"
  Assert-PackBackupDrive -BackupRoot ([string]$Ctx.backup_root)
  $script1 = 'recorder-gate.ps1'
  $args1 = @(Get-PackLegacyCommon -Ctx $Ctx) + @('-RigPrefer', [string]$Ctx.rig_prefer, '-SecondReference')
  if ($Gate -eq 'kill') {
    $script1 = 'kill-gate.ps1'
    $args1 += @('-Kills', "$(if ($quick) { $QuickKills } else { $FullKills })")
    $minutes = 0
  } else {
    $args1 += @('-Gate', $Gate)
    if ($minutes -gt 0) { $args1 += @('-Minutes', ($minutes.ToString([Globalization.CultureInfo]::InvariantCulture))) }
  }
  if ($Beside) {
    $bc = [ordered]@{
      mode = 'live-batch'; engines_dir = $paths.Engines; models_dir = $paths.Models; clip = (Join-Path $paths.BenchClips 'bench-en.wav')
      live_variant = 'avx2'; batch_variant = 'avx2'; batch_device = 'cpu'; batch_threads = 2; live_threads = 2; model = 'medium'; lang = 'en'
    }
    foreach ($k in @($Beside.Keys)) { $bc[$k] = $Beside[$k] }
    $bcFile = Join-Path $gdir ("beside-config$RunTag.json")
    [IO.File]::WriteAllText($bcFile, ($bc | ConvertTo-Json -Depth 4), (New-Object Text.UTF8Encoding($false)))
    $args1 += @('-BesideScript', (ConvertTo-PackArg (Join-Path $paths.Lib 'beside-load.ps1')), '-BesideConfig', (ConvertTo-PackArg $bcFile))
  }
  $th = $null
  if ($Thermal) { $th = Start-PackThermal -OutCsv (Join-Path $gdir 'thermal.csv') }
  $startedAt = Get-Date
  try {
    $e = Invoke-PackLegacyGate -Ctx $Ctx -Script $script1 -Arguments $args1 -EvidenceName ("$Gate-$([string]$Ctx.profile).txt") -TimeoutMinutes ([int]($minutes + 120))
  } finally {
    $thermalRes = $null
    if ($th) { $thermalRes = Stop-PackThermal -Handle $th }
  }
  if ($e.TimedOut) { return (New-PackResult -Id $Id -Status 'INVALID' -Headline "the $Gate gate did not finish in time" -Evidence @($e.Log) -Quick $quick) }
  if ($RunTag -and $e.Evidence) {
    $tagged = Join-Path (Split-Path -Parent $e.Evidence) ("$Gate-$([string]$Ctx.profile)-$RunTag.txt")
    Copy-Item -LiteralPath $e.Evidence -Destination $tagged -Force
    $e.Evidence = $tagged
  }
  $evLines = @()
  if ($e.Evidence) { $evLines = @(Get-Content -LiteralPath $e.Evidence -Encoding UTF8) }
  # A refusal before the run (rig, overlay tools, RAM) is reported by the gate's crash evidence; pass its reason on.
  $reason = @($evLines | Where-Object { $_ -cmatch '^(RIG REFUSED|RIG SELF-TEST FAILED|Overlay tools are running|Only [0-9.]+ GB|An engine build)' } | Select-Object -First 1)
  if ($reason.Count -eq 0) { $reason = @($evLines | Where-Object { $_ -cmatch 'the \w+ gate stopped on an error: (RIG REFUSED|RIG SELF-TEST FAILED|Overlay tools are running|Only [0-9.]+ GB|An engine build)' } | Select-Object -First 1) }
  if ($reason.Count -gt 0 -and $e.ExitCode -ne 0) {
    $msg = [string]$reason[0]
    if ($msg -match 'stopped on an error: (.+)$') { $msg = $Matches[1] }
    return (New-PackResult -Id $Id -Status (Get-PackErrorStatus $msg) -Headline $msg -Evidence @($e.Evidence) -Quick $quick)
  }
  $out = Get-PackRecorderOutcome -ExitCode $e.ExitCode -EvidenceLines $evLines
  $runDir = Get-PackRunDirFromEvidence -EvidenceLines $evLines
  $rigKind = Get-PackRigKindFromEvidence -EvidenceLines $evLines
  $extra = New-Object System.Collections.Generic.List[string]
  $extra.Add('')
  $extra.Add('gate pack additions:')
  $extra.Add(('  rig: {0}' -f $rigKind))
  if ($thermalRes) { foreach ($l in $thermalRes.Lines) { $extra.Add($l) } }
  $det = $null
  if ($runDir -and (Test-Path -LiteralPath $runDir)) {
    try {
      $cond = ('rig {0}; machine {1}' -f $rigKind, $Ctx.machine)
      $det = Get-PackRunDetectors -RunDir $runDir -Testbench $paths.Testbench -Conditions $cond
      foreach ($l in $det.Lines) { $extra.Add($l) }
    } catch { $extra.Add("  detector rates: could not be worked out ($($_.Exception.Message))") }
  }
  $status = $out.Status
  $notes = @()
  if ($thermalRes -and $thermalRes.Summary.Usable -and $thermalRes.Summary.AcLost) {
    # Running on battery makes a soak or load run say nothing about the room: repeat on mains power.
    $status = 'INVALID'
    $notes += 'the laptop ran on battery during this run; repeat it on mains power'
  }
  $addFile = Join-Path $gdir ($(if ($RunTag) { "$Id-$RunTag-additions.txt" } else { "$Id-additions.txt" }))
  Set-Content -LiteralPath $addFile -Value @($extra) -Encoding UTF8
  $head = $(if ($out.VerdictLine) { $out.VerdictLine -replace '^verdict:\s*', '' } else { $status })
  if ($out.Status -eq 'FAIL') {
    $failedChecks = @(Get-PackFailedChecks -EvidenceLines $evLines)
    if ($failedChecks.Count -gt 0) { $head = ($head + ': ' + ($failedChecks -join '; ')) }
  }
  if ($rigKind -eq 'vb-cable-fallback') { $head = "$head [VB-CABLE fallback: does not close the hardware-interface rows]" }
  # Unmatched stretches of the app's recording: shared when the reference capture has them too (the source
  # chain broke), app-only when it has not (the recorder's own).
  $sharedBreaks = @($evLines | Where-Object { $_ -match 'unmatched stretch:.*also in the reference capture: True' }).Count
  $appOnlyBreaks = @($evLines | Where-Object { $_ -match 'unmatched stretch:.*also in the reference capture: False' }).Count
  $measured = [ordered]@{ rig = $rigKind; mirror_same_drive = [bool](Test-PackMirrorSameDrive -WorkPath ([string]$Ctx.work_dir) -BackupPath ([string]$Ctx.backup_root)); minutes = $minutes; exit_code = $e.ExitCode; run_dir = $runDir; shared_breaks = $sharedBreaks; app_only_breaks = $appOnlyBreaks }
  if ($thermalRes -and $thermalRes.Summary.Usable) {
    $measured['thermal'] = [ordered]@{ clock_drop_pct = $thermalRes.Summary.ClockDropPct; ac_lost = $thermalRes.Summary.AcLost; event37 = $thermalRes.Event37; limit_below_100 = $thermalRes.Summary.LimitBelow100 }
  }
  if ($det) { $measured['detectors'] = $det.Rates }
  $evid = @($e.Evidence, $addFile) | Where-Object { $_ }
  New-PackResult -Id $Id -Status $status -Headline $head -Measured $measured -Threshold $Threshold -Rig (Get-PackRigLabel -Rig ([pscustomobject]@{ Kind = $(if ($rigKind -eq 'loopback') { 'loopback' } else { 'vb-cable' }); Fallback = ($rigKind -eq 'vb-cable-fallback'); Instance = '' })) -Evidence @($evid) -Notes $notes -Quick $quick -SharedBreaks $sharedBreaks
}

# The device the batch engine runs on beside the recorder: the integrated graphics adapter when the
# vulkan engine lists one (as the app's profile would use it), else the CPU. Returns the keys for the
# -Beside table of Invoke-PackRecorderGate.
# The thread counts of the machine as the app sees them: physical and logical processors. Under an
# emulation profile the engine reports the emulated processors in `info` (logical_processors_allowed).
function Get-PackMachineThreads {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Ctx)
  $paths = Get-PackPaths -Ctx $Ctx
  $fp = Read-PackJson -Path (Join-Path ([string]$Ctx.work_dir) 'fingerprint.json')
  $phys = $(if ($fp) { [int]$fp.physical_cores } else { 4 })
  $logi = $(if ($fp) { [int]$fp.logical_processors } else { 4 })
  if ([string]$Ctx.profile -ne 'laptop' -and $paths.Emulate -and (Test-Path -LiteralPath $paths.Emulate)) {
    try {
      $ae = Join-Path $paths.Engines 'wb-engine-avx2-x86_64-pc-windows-msvc.exe'
      $ri = Invoke-PackNative -FilePath $paths.Emulate -ArgumentList @('--profile', [string]$Ctx.profile, '--', $ae, 'info') -TimeoutSeconds 60
      foreach ($l in @($ri.Output)) {
        if ($l -notmatch '^\{') { continue }
        $j = $l | ConvertFrom-Json
        if ($j.PSObject.Properties['event'] -and $j.event -eq 'info' -and [int]$j.logical_processors_allowed -ge 1) { $logi = [int]$j.logical_processors_allowed; $phys = [math]::Min($phys, $logi) }
      }
    } catch { }
  }
  [pscustomobject]@{ Physical = [math]::Max(1, $phys); Logical = [math]::Max(1, $logi) }
}

# What runs beside the recorder, as the app would run it on this machine: the model, the device and the CPU
# threads the quick check chose (capability.json); without one, the app's own counts (batch: physical cores
# minus one, live: half the logical processors). Returns keys for the -Beside table.
function Get-PackBesideProduction {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Ctx)
  $paths = Get-PackPaths -Ctx $Ctx
  $t = Get-PackMachineThreads -Ctx $Ctx
  $batch = [math]::Max(1, $t.Physical - 1)
  $live = [math]::Max(1, [int][math]::Floor($t.Logical / 2))
  $model = 'medium'
  $note = 'model medium (no quick-check result on this laptop yet)'
  $cap = Read-PackJson -Path (Join-Path ([string]$Ctx.work_dir) 'gates\quick-check\capability.json')
  try {
    $m = [string]$cap.decision.languages.en.rolling.model
    if ($m -and (Test-Path -LiteralPath (Join-Path $paths.Models ('ggml-{0}.bin' -f $m)))) { $model = $m; $note = "model $m (the quick check's rolling model for English)" }
    $ct = $cap.decision.config.cpu_threads
    if ($null -ne $ct -and [int]$ct -ge 1 -and [string]$cap.decision.config.config.kind -eq 'gpu') { $batch = [int]$ct }
  } catch { }
  @{ model = $model; batch_threads = $batch; live_threads = $live; production_note = ('{0}; batch {1} threads, live {2} threads' -f $note, $batch, $live) }
}

function Get-PackBesideDevice {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Ctx)
  $paths = Get-PackPaths -Ctx $Ctx
  $prod = Get-PackBesideProduction -Ctx $Ctx
  $cpu = @{ batch_variant = 'avx2'; batch_device = 'cpu'; batch_threads = $prod.batch_threads; live_threads = $prod.live_threads; model = $prod.model; device_note = ('CPU (avx2), {0}' -f $prod.production_note) }
  $vk = Join-Path $paths.Engines 'wb-engine-vulkan-x86_64-pc-windows-msvc.exe'
  if (-not (Test-Path -LiteralPath $vk) -or [string]$Ctx.profile -ne 'laptop') { return $cpu }
  try {
    $r = Invoke-PackNative -FilePath $vk -ArgumentList @('info') -TimeoutSeconds 60
    foreach ($l in @($r.Output)) {
      if ($l -notmatch '^\{') { continue }
      $j = $l | ConvertFrom-Json
      if ($j.PSObject.Properties['event'] -and $j.event -eq 'info') {
        $d = @($j.devices | Where-Object { [string]$_.backend -match 'Vulkan' -and [string]$_.kind -match 'integrated|igpu' } | Select-Object -First 1)
        if ($d.Count -gt 0) { return @{ batch_variant = 'vulkan'; batch_device = ('gpu:{0}' -f [int]$d[0].index); batch_threads = $prod.batch_threads; live_threads = $prod.live_threads; model = $prod.model; device_note = ('integrated graphics, vulkan device {0} ({1}), {2}' -f $d[0].index, $d[0].description, $prod.production_note) } }
      }
    }
  } catch { }
  $cpu
}

Export-ModuleMember -Function Get-PackFailedChecks, Get-PackAppTimeFindings, Get-PackMachineThreads, Get-PackBesideProduction, Get-PackBesideDevice, Get-PackRecorderOutcome, Get-PackRunDirFromEvidence, Get-PackRigKindFromEvidence, Get-PackCompareOffsetMs, Get-PackScanFindings, Get-PackRunDetectors, Invoke-PackRecorderGate
