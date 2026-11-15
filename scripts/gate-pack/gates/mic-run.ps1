# Gate mic-run: the app (gate build) and two independent reference captures (cpal and raw WASAPI) record the
# laptop's built-in microphone while public speech (English and Hindi test clips) plays aloud from the
# laptop's own speakers, for 2 hours (15 minutes under -Quick). Two scripted windows test the recorder's
# detectors: a digital mute of the microphone (the endpoint delivers zeros) and a room silence (the source is
# silent); neither is a dropout. Verdict: the app equals both references and the recorder counted no overrun.
# The detector rates (zero_gap, repeated_block) and what each detector did in each window are information.
# The known-speech recording stays on this laptop (work\audio is never in the zip). It plays aloud, so it
# needs a person in a quiet room and is never run unattended. The speaker volume and the microphone mute are
# recorded as changes and put back at the end, on an abort and by run-gates.ps1 -Cleanup.
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'GateRun.psm1') -Force
Import-Module (Join-Path $lib 'CableRig.psm1') -Force
Import-Module (Join-Path $lib 'PackNative.psm1') -Force
Import-Module (Join-Path $lib 'LoopbackRig.psm1') -Force
Import-Module (Join-Path $lib 'RefVerdict.psm1') -Force
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force
Import-Module (Join-Path $lib 'MicRun.psm1') -Force
$paths = Get-PackPaths -Ctx $ctx

function Read-RunJson([string]$Path) { Read-PackJson -Path $Path }
function Get-Prop($o, [string]$n, $d = $null) { if ($null -eq $o) { return $d }; $p = $o.PSObject.Properties[$n]; if ($null -eq $p) { return $d }; $p.Value }

Invoke-PackGateBody -Ctx $ctx -Id 'mic-run' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $profile = [string]$ctx.profile
  $thr = 'the app equals both independent references of the built-in microphone sample for sample; the recorder counted no overrun; detector firings, false alarms and the two scripted windows are reported as information'
  if ($profile -ne 'laptop') { throw 'RIG REFUSED: the microphone run needs the laptop''s own microphone and speakers; it is not run under an emulation profile' }
  if ([bool]$ctx.unattended) { throw 'RIG REFUSED: the microphone run plays speech aloud and needs a person in a quiet room; it is not run unattended' }
  foreach ($f in @($paths.GateExe, $paths.Testbench, (Join-Path $paths.BenchClips 'bench-en.wav'), (Join-Path $paths.BenchClips 'bench-hi.wav'))) {
    if (-not (Test-Path -LiteralPath $f)) { throw "the pack is missing $f (run run-gates.ps1 -Verify)" }
  }
  $overlay = @(Get-WbOverlayTools)
  if ($overlay.Count -gt 0) { throw ('Overlay tools are running: ' + ($overlay -join ', ') + '. Close them and run the microphone test again.') }
  $ac = Get-PackMainsPower
  if ($ac -eq $false) { throw 'RIG REFUSED: the laptop is on battery; connect the charger for the microphone run' }
  $free = Get-WbFreeRamGB
  if ($free -lt [double]$ctx.min_free_ram_gb) { throw ('Only {0:n1} GB of RAM free; close other programs ({1:n0} GB needed)' -f $free, [double]$ctx.min_free_ram_gb) }
  $plan = Get-PackMicPlan -Quick $quick
  $windows = @(Get-PackMicWindows -Plan $plan)

  # Devices: the built-in microphone and the speakers Windows plays through.
  $devices = Get-PackTestbenchDevices -Testbench $paths.Testbench
  $mic = Select-PackMicDevice -Inputs @($devices.inputs)
  if ($null -eq $mic) { throw 'RIG REFUSED: no built-in microphone was found (virtual cables and audio interfaces are not used for this test)' }
  $speaker = [string](Get-Prop $devices 'default_output' '')
  if (-not $speaker) { throw 'RIG REFUSED: Windows has no default playback device (speakers) for the microphone test' }
  if ($speaker -match 'CABLE|VB-Audio|Virtual') { throw "RIG REFUSED: the default playback device is a virtual cable ($speaker); set the laptop's speakers as the default and run again" }
  $micName = [string]$mic.name
  $free2 = (Get-PSDrive -Name (Split-Path -Qualifier ([string]$ctx.work_dir)).TrimEnd(':') -ErrorAction SilentlyContinue).Free
  $needGB = $(if ($quick) { 2.0 } else { 7.0 })
  if ($null -ne $free2 -and ($free2 / 1GB) -lt $needGB) { throw ('Only {0:n1} GB of disk space free on the pack drive; {1:n0} GB are needed for the microphone recordings' -f ($free2 / 1GB), $needGB) }

  $minutes = [math]::Round($plan.Seconds / 60.0)
  $answer = Request-PackPrompt -Ctx $ctx -TimeoutMinutes 60 -Message ("MICROPHONE TEST ($minutes minutes)`r`nThis test plays speech ALOUD from this laptop's speakers and records it with the built-in microphone ($micName).`r`nThe room must be quiet. Do not talk, do not move the laptop, leave the charger connected.`r`nThe speaker volume is set to a moderate level and put back at the end. Press Enter to start.")
  if ($answer -match '^(?i)(skip|no|n)$') { throw 'RIG REFUSED: the microphone test was skipped by the person at the laptop' }

  # What is on the machine that can change the sound.
  $cond = New-Object System.Collections.Generic.List[string]
  $proc = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'Realtek|Rtk|Nahimic|Dolby|Waves|SmartSound|A-Volute|Sonic' } | ForEach-Object { $_.ProcessName } | Sort-Object -Unique)
  $cond.Add('audio software running: ' + $(if ($proc.Count -gt 0) { $proc -join ', ' } else { 'none of Realtek, Nahimic, Dolby, Waves, Sonic Studio found as a process' }))
  try { $ss = @(Get-CimInstance Win32_PnPEntity -ErrorAction Stop | Where-Object { $_.Name -match 'Smart Sound|Realtek.*Audio|Dolby' } | ForEach-Object { $_.Name } | Sort-Object -Unique); $cond.Add('audio devices of interest: ' + $(if ($ss.Count -gt 0) { $ss -join '; ' } else { 'none' })) } catch { $cond.Add('audio devices of interest: could not be listed') }
  $cond.Add('Windows audio enhancements: not exposed to the pack; the format the recorder opened is below')

  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $run = Join-Path (Join-Path ([string]$ctx.work_dir) 'audio') "mic-$stamp"
  $rec = Join-Path $run 'rec'
  $backup = Join-Path ([string]$ctx.backup_root) "mic-$stamp"
  foreach ($d in @($run, $rec)) { New-Item -ItemType Directory -Force -Path $d | Out-Null }
  $gdir = [string]$ctx.gate_dir
  $source = Join-Path $run 'speech-source.wav'
  $clips = (Join-Path $paths.BenchClips 'bench-en.wav') + ',' + (Join-Path $paths.BenchClips 'bench-hi.wav')
  $mix = Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('mix', '--out', $source, '--clips', $clips, '--seconds', ([string]$plan.Seconds), '--pause-s', '2', '--silent-from-s', ([string]$plan.SilenceAtS), '--silent-for-s', ([string]$plan.SilenceForS)) -TimeoutSeconds 900
  if ($mix.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $source)) { throw ('wb-testbench mix failed: ' + ((@($mix.Output) + @($mix.Error)) -join ' ')) }

  # Speaker volume: moderate, restored. Mute: recorded before it is touched.
  $vol = Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('volume', '--device', $speaker, '--flow', 'render')
  $volBefore = $null
  try { $volBefore = [double]((($vol.Output -join "`n") | ConvertFrom-Json).was_level) } catch { }
  if ($null -ne $volBefore) {
    Add-PackChange -WorkDir ([string]$ctx.work_dir) -Kind 'render-volume' -Description ("speaker volume of '$speaker' (was $([math]::Round($volBefore * 100)) percent)") -Restore @{ device = $speaker; level_before = $volBefore.ToString([Globalization.CultureInfo]::InvariantCulture) }
    $setv = Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('volume', '--device', $speaker, '--flow', 'render', '--level', '0.5')
    if ($setv.ExitCode -ne 0) { $cond.Add('speaker volume could not be set: the loudness is whatever it was') } else { $cond.Add('speaker volume set to 50 percent (put back at the end)') }
  } else { $cond.Add('speaker volume could not be read: left as it is') }
  $mu = Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('mute', '--device', $micName, '--flow', 'capture')
  $muteBefore = 'off'
  try { if ([bool]((($mu.Output -join "`n") | ConvertFrom-Json).was_muted)) { $muteBefore = 'on' } } catch { }
  Add-PackChange -WorkDir ([string]$ctx.work_dir) -Kind 'mic-mute' -Description ("microphone mute of '$micName' (was $muteBefore)") -Restore @{ device = $micName; state_before = $muteBefore }
  if ($muteBefore -eq 'on') {
    $um = Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('mute', '--device', $micName, '--flow', 'capture', '--state', 'off')
    $cond.Add('the microphone was muted in Windows when the test began: unmuted for the test, put back at the end')
  }

  $stopFile = Join-Path $run 'STOP'; $refStop = Join-Path $run 'STOP-REF'; $ref2Stop = Join-Path $run 'STOP-REF2'; $goFile = Join-Path $run 'GO'
  $env:WB_GATE_ROOT = $rec; $env:WB_GATE_BACKUP = $backup; $env:WB_GATE_START = "Gate mic $stamp"; $env:WB_GATE_DEVICE = $micName
  $env:WB_GATE_STOP_FILE = $stopFile; $env:WB_GATE_REPORT = Join-Path $run 'stop-report.json'; $env:WB_GATE_STATUS_FILE = Join-Path $run 'status.json'
  foreach ($v in 'WB_GATE_RECOVERY', 'WB_GATE_PANIC_AT_S', 'WB_GATE_RECORDER_PANIC_AT_S', 'WB_GATE_PART_SECONDS') { Set-Item -Path "Env:\$v" -Value '' }
  $app = $null; $refProc = $null; $ref2Proc = $null; $play = $null
  $windowLog = New-Object System.Collections.Generic.List[object]
  $finished = $false
  $goAt = $null
  try {
    $app = Start-Process -FilePath $paths.GateExe -PassThru -WindowStyle Hidden
    $deadline = (Get-Date).AddSeconds(120)
    $st = $null
    while ($true) {
      $st = Read-RunJson (Join-Path $run 'status.json')
      if ($st -and $st.recorder -and $st.recorder.state -eq 'recording') { break }
      if ($app.HasExited) { throw "the app ended before recording started (exit $($app.ExitCode))" }
      if ((Get-Date) -gt $deadline) { throw 'recording did not start within 120 s' }
      Start-Sleep -Milliseconds 500
    }
    $sessionDir = [string]$st.recorder.session_dir
    $refDir = Join-Path $run 'reference'; $ref2Dir = Join-Path $run 'reference2'
    $q = { param($s) ConvertTo-PackArg $s }
    $refProc = Start-Process -FilePath $paths.Testbench -ArgumentList @('capture', '--device', (& $q $micName), '--dir', (& $q $refDir), '--stop-file', (& $q $refStop), '--go-file', (& $q $goFile), '--report', (& $q (Join-Path $run 'reference.json'))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run 'reference.out') -RedirectStandardError (Join-Path $run 'reference.err')
    $ref2Proc = Start-Process -FilePath $paths.Testbench -ArgumentList @('capture-wasapi', '--device', (& $q $micName), '--dir', (& $q $ref2Dir), '--stop-file', (& $q $ref2Stop), '--go-file', (& $q $goFile), '--report', (& $q (Join-Path $run 'reference2.json'))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run 'reference2.out') -RedirectStandardError (Join-Path $run 'reference2.err')
    Start-Sleep -Seconds 1
    $play = Start-Process -FilePath $paths.Testbench -ArgumentList @('play', '--device', (& $q $speaker), '--file', (& $q $source), '--go-file', (& $q $goFile), '--report', (& $q (Join-Path $run 'play.json'))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run 'play.out') -RedirectStandardError (Join-Path $run 'play.err')
    Start-Sleep -Seconds 2
    Set-Content -LiteralPath $goFile -Value 'go' -Encoding ascii
    $goAt = Get-Date
    Write-PackGateLog -Ctx $ctx -Message ("microphone run started: {0} minutes; microphone '{1}'; speakers '{2}'" -f $minutes, $micName, $speaker)
    $muted = $false; $muteDone = $false; $lastLog = Get-Date
    while (-not $play.HasExited) {
      $el = ((Get-Date) - $goAt).TotalSeconds
      if (-not $muted -and -not $muteDone -and $el -ge $plan.MuteAtS) {
        $r = Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('mute', '--device', $micName, '--flow', 'capture', '--state', 'on')
        $muted = ($r.ExitCode -eq 0)
        $windowLog.Add([ordered]@{ event = 'mute on'; ok = $muted; elapsed_s = [math]::Round($el, 1); at = (Get-Date).ToString('o') })
      }
      if ($muted -and $el -ge ($plan.MuteAtS + $plan.MuteForS)) {
        $r = Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('mute', '--device', $micName, '--flow', 'capture', '--state', 'off')
        $muted = -not ($r.ExitCode -eq 0); $muteDone = $true
        $windowLog.Add([ordered]@{ event = 'mute off'; ok = ($r.ExitCode -eq 0); elapsed_s = [math]::Round($el, 1); at = (Get-Date).ToString('o') })
      }
      if (((Get-Date) - $lastLog).TotalSeconds -ge 300) { $lastLog = Get-Date; Write-PackGateLog -Ctx $ctx -Message ('microphone run: {0:n0} of {1:n0} minutes' -f ($el / 60.0), ($plan.Seconds / 60.0)) }
      if ($app.HasExited) { throw 'the app exited during the microphone run' }
      Start-Sleep -Seconds 1
    }
    if ($muted) { [void](Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('mute', '--device', $micName, '--flow', 'capture', '--state', 'off')) }
    Start-Sleep -Seconds 2
    $lastStatus = Read-RunJson (Join-Path $run 'status.json')
    Set-Content -LiteralPath $refStop -Value 'stop'; [void]$refProc.WaitForExit(120000)
    Set-Content -LiteralPath $ref2Stop -Value 'stop'; [void]$ref2Proc.WaitForExit(120000)
    Set-Content -LiteralPath $stopFile -Value 'stop'
    if (-not $app.WaitForExit(900000)) { throw 'the app did not stop within 15 minutes' }
    $finished = $true
  } finally {
    if (-not $finished) {
      $left = @(Stop-WbRigLeftovers -Process @($app, $play, $refProc, $ref2Proc) -StopFile @($stopFile, $refStop, $ref2Stop) -GraceSeconds 45)
      foreach ($l in $left) { Write-PackGateLog -Ctx $ctx -Message "cleanup after an error: $l" }
    }
    # Put the microphone and the speakers back whatever happened.
    [void](Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('mute', '--device', $micName, '--flow', 'capture', '--state', $muteBefore))
    if ($null -ne $volBefore) { [void](Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('volume', '--device', $speaker, '--flow', 'render', '--level', $volBefore.ToString([Globalization.CultureInfo]::InvariantCulture))) }
    foreach ($v in 'WB_GATE_ROOT', 'WB_GATE_BACKUP', 'WB_GATE_START', 'WB_GATE_DEVICE', 'WB_GATE_STOP_FILE', 'WB_GATE_REPORT', 'WB_GATE_STATUS_FILE') { Remove-Item "Env:\$v" -ErrorAction SilentlyContinue }
  }
  Write-PackJson -Path (Join-Path $gdir 'mic-windows.json') -Value ([ordered]@{ plan = $plan; windows = $windows; log = @($windowLog.ToArray()); microphone = $micName; speakers = $speaker })

  # Judging: the app against both references, the recorder's own counters, the detectors.
  $report = Read-RunJson (Join-Path $run 'stop-report.json')
  $stopped = Get-Prop $report 'stopped'
  $recd = Get-Prop $stopped 'recorder'
  if ($null -eq $recd) { return (New-PackResult -Id 'mic-run' -Status 'INVALID' -Headline 'the app wrote no stop report; there is nothing to judge' -Threshold $thr -Quick $quick) }
  $sessionDir = [string]$st.recorder.session_dir
  $cmpFiles = @(
    @('compare.json', $sessionDir, (Join-Path $run 'reference')),
    @('compare2.json', $sessionDir, (Join-Path $run 'reference2')),
    @('compare-refs.json', (Join-Path $run 'reference'), (Join-Path $run 'reference2')))
  foreach ($c in $cmpFiles) {
    if ((Test-Path -LiteralPath $c[1]) -and (Test-Path -LiteralPath $c[2])) { [void](Invoke-PackNative -FilePath $paths.Testbench -ArgumentList @('compare', '--app', $c[1], '--reference', $c[2], '--report', (Join-Path $run $c[0])) -TimeoutSeconds 3600) }
  }
  $rv = Get-PackRefVerdict -AppCpal (Read-RunJson (Join-Path $run 'compare.json')) -AppWasapi (Read-RunJson (Join-Path $run 'compare2.json')) -CpalWasapi (Read-RunJson (Join-Path $run 'compare-refs.json'))
  $overruns = [int](Get-Prop $recd 'overrun_samples' 0)
  $xruns = [int](Get-Prop $recd 'xruns' 0)
  $status = $rv.Verdict
  $why = $rv.Reason
  if ($overruns -gt 0) { $status = 'FAIL'; $why = ("the recorder counted {0} overrun samples; {1}" -f $overruns, $rv.Reason) }
  $firings = @()
  $logf = @(Get-ChildItem -LiteralPath $rec -Recurse -Filter 'session.log' -ErrorAction SilentlyContinue | Select-Object -First 1)
  if ($logf.Count -gt 0) { $firings = @(ConvertFrom-PackSessionLog -Lines @(Get-Content -LiteralPath $logf[0].FullName -Encoding UTF8)) }
  $offA = Get-PackCompareOffsetMs -Compare (Read-RunJson (Join-Path $run 'compare.json'))
  # The app clock minus the source clock: the reference capture starts with the source (the go file), so its offset is the app's.
  $offset = $(if ($null -ne $offA) { [double]$offA } else { 0.0 })
  $det = $null
  try { $det = Get-PackRunDetectors -RunDir $run -Testbench $paths.Testbench -Conditions ("built-in microphone '{0}'; machine {1}" -f $micName, $ctx.machine) -ExcludeWindows $windows -WindowOffsetMs $offset } catch { }
  $treat = @(Get-PackWindowTreatment -Firings $firings -Windows $windows -OffsetMs $offset)
  $inputInfo = ''
  try { $inputInfo = ('{0} Hz, {1} channel(s), {2}' -f $mic.sample_rate, $mic.channels, $mic.sample_format) } catch { }

  $ev = Join-Path $gdir 'mic-run.txt'
  $L = New-Object System.Collections.Generic.List[string]
  $L.Add('Microphone run (gate pack)')
  $L.Add(('machine: {0}; profile: {1}; date: {2}; {3} minutes{4}' -f $ctx.machine, $profile, (Get-Date -Format 'yyyy-MM-dd HH:mm'), $minutes, $(if ($quick) { ' (QUICK: the full run is 120 minutes)' } else { '' })))
  $L.Add(('microphone: {0}; format the recorder opens: {1}; speakers: {2}' -f $micName, $inputInfo, $speaker))
  foreach ($c in $cond) { $L.Add("  $c") }
  $L.Add(('windows (source clock): digital mute {0} s for {1} s; room silence {2} s for {3} s' -f $plan.MuteAtS, $plan.MuteForS, $plan.SilenceAtS, $plan.SilenceForS))
  foreach ($w in $windowLog) { $L.Add(('  window log: {0} at {1} s ({2})' -f $w['event'], $w['elapsed_s'], $(if ($w['ok']) { 'done' } else { 'FAILED to change the mute' }))) }
  $L.Add(('recorder: overrun samples {0}, xruns {1}' -f $overruns, $xruns))
  foreach ($l in $rv.Lines) { $L.Add($l) }
  $L.Add(('reference verdict: {0} ({1}): {2}' -f $rv.Verdict, $rv.Class, $rv.Reason))
  if ($det) { foreach ($l in $det.Lines) { $L.Add($l) } }
  $L.Add('what each detector did in each scripted window:')
  foreach ($l in @(Get-PackWindowLines -Treatment $treat)) { $L.Add($l) }
  $L.Add("audio and reports: $run (not in the repository; the known-speech recording stays on this laptop and is not in the zip)")
  $L.Add("verdict: $status")
  Set-Content -LiteralPath $ev -Value @($L) -Encoding UTF8
  $measured = [ordered]@{ minutes = $minutes; microphone = $micName; overrun_samples = $overruns; xruns = $xruns; reference_class = $rv.Class; windows = @($treat | ForEach-Object { [ordered]@{ window = $_.Window; zero_gap = $_.zero_gap; repeated_block = $_.repeated_block } }) }
  if ($det) { $measured['detectors'] = $det.Rates }
  New-PackResult -Id 'mic-run' -Status $status -Headline ("{0} (microphone run, {1} minutes)" -f $why, $minutes) -Measured $measured -Threshold $thr -Rig ("built-in microphone: $micName") -Evidence @($ev, (Join-Path $gdir 'mic-windows.json')) -Notes @($cond.ToArray()) -Quick $quick
}
