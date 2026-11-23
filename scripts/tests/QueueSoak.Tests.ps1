# Tests for the queue-soak gate (scripts/lib/QueueSoak.psm1, scripts/gates/queue-soak.ps1, scripts/gate-pack/gates/queue-soak.ps1):
# the capability file and the PENDING path, the plan, the transcript checks, the overlap of decoding with the second
# recording, the verdict rules (fake inputs), the runner order in the real registry, and the guard on the app data folder.
# No engine, app or audio device is started.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/QueueSoak.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PackCore.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\QueueSoak.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) { if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ } }
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('qs-tests-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
function Write-Text([string]$Path, [string]$Text) { New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null; [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding($false))) }

try {
  # ---- settings: quick under about 25 minutes, full about 60 -------------------------------------------------------
  $q = Get-WbQueueSoakSettings -Quick
  $f = Get-WbQueueSoakSettings
  Check 'quick mode: two sessions of 5 minutes with 2-minute parts' ($q.Mode -eq 'quick' -and $q.Session1Seconds -eq 300 -and $q.Session2Seconds -eq 300 -and $q.PartSeconds -eq 120)
  Check 'full mode: 25 and 20 minutes with the app''s own part length' ($f.Mode -eq 'full' -and $f.Session1Seconds -eq 1500 -and $f.Session2Seconds -eq 1200 -and $f.PartSeconds -eq 0)
  Check 'the recording time of quick mode leaves room for the rig, the wait for decoding and the analysis in about 25 minutes' (($q.Session1Seconds + $q.Session2Seconds) / 60.0 -le 12)

  # ---- the capability file: no file, no gate (PENDING with a reason) -----------------------------------------------
  $missing = Get-WbQueueSoakCapability -Path (Join-Path $tmp 'gates\quick-check\capability.json')
  Check 'no capability.json: not found, and the reason names the quick check' (-not $missing.Found -and $missing.Reason -match 'quick check' -and $missing.Reason -match 'capability\.json')
  Write-Text (Join-Path $tmp 'empty.json') ''
  Check 'an empty capability.json is not usable' (-not (Get-WbQueueSoakCapability -Path (Join-Path $tmp 'empty.json')).Found)
  Write-Text (Join-Path $tmp 'bad.json') '{ not json'
  $bad = Get-WbQueueSoakCapability -Path (Join-Path $tmp 'bad.json')
  Check 'a capability.json that is not JSON is not usable and says so' (-not $bad.Found -and $bad.Reason -match 'not valid JSON')
  Write-Text (Join-Path $tmp 'nodec.json') '{ "version": 1 }'
  Check 'a capability.json without a decision is not usable' (-not (Get-WbQueueSoakCapability -Path (Join-Path $tmp 'nodec.json')).Found)
  Write-Text (Join-Path $tmp 'ok.json') '{ "decision": { "tier": "C", "process_while_recording": true } }'
  $ok = Get-WbQueueSoakCapability -Path (Join-Path $tmp 'ok.json')
  Check 'a capability.json with a decision is usable and says whether processing while recording was cleared' ($ok.Found -and $ok.ProcessWhileRecording -eq $true)
  Write-Text (Join-Path $tmp 'no.json') '{ "decision": { "tier": "R", "process_while_recording": false } }'
  Check 'a check that did not clear processing while recording is read as false' ((Get-WbQueueSoakCapability -Path (Join-Path $tmp 'no.json')).ProcessWhileRecording -eq $false)

  # ---- the pack wrapper, run for real: without a kept capability.json it answers PENDING and starts nothing --------
  $work = Join-Path $tmp 'work'
  $gdir = Join-Path $work 'gates\queue-soak'
  $ctx = [ordered]@{
    pack_root = $repo; work_dir = $work; gate_id = 'queue-soak'; gate_dir = $gdir; profile = 'emu-low'; emulate = ''; role = 'low'; quick = $true
    machine = 'TEST'; lock_dir = $work; min_free_ram_gb = 7; uptime_min = 90; boot_time = ''; admin = $false; rig_prefer = 'loopback'
    mirror_root = ''; backup_root = (Join-Path $tmp 'backup'); unattended = $true
  }
  New-Item -ItemType Directory -Force -Path $work | Out-Null
  Write-Text (Join-Path $work 'ctx.json') ($ctx | ConvertTo-Json)
  $resFile = Join-Path $work 'result.json'
  $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $repo 'scripts\gate-pack\gates\queue-soak.ps1')`"", '-CtxFile', "`"$(Join-Path $work 'ctx.json')`"", '-ResultFile', "`"$resFile`"") -PassThru -WindowStyle Hidden -Wait
  $res = $null
  if (Test-Path -LiteralPath $resFile) { $res = Get-Content -LiteralPath $resFile -Raw | ConvertFrom-Json }
  Check 'the wrapper wrote a result with no capability.json from the quick check' ($null -ne $res -and $res.id -eq 'queue-soak')
  Check 'that result is PENDING, never FAIL, and says what is missing' ($res -and $res.status -eq 'PENDING' -and $res.headline -match 'quick check' -and $res.headline -match 'capability\.json')
  Check 'nothing was started: no run folder, no evidence folder' (-not (Test-Path -LiteralPath (Join-Path $gdir 'evidence')) -and -not (Test-Path -LiteralPath (Join-Path $work 'audio')))
  Remove-Item -LiteralPath $resFile -Force
  Write-Text (Join-Path $work 'gates\quick-check\capability.json') ''
  $p = Start-Process -FilePath $ps -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $repo 'scripts\gate-pack\gates\queue-soak.ps1')`"", '-CtxFile', "`"$(Join-Path $work 'ctx.json')`"", '-ResultFile', "`"$resFile`"") -PassThru -WindowStyle Hidden -Wait
  $res = $(if (Test-Path -LiteralPath $resFile) { Get-Content -LiteralPath $resFile -Raw | ConvertFrom-Json } else { $null })
  Check 'an empty capability.json from the quick check is PENDING too' ($res -and $res.status -eq 'PENDING' -and $res.headline -match 'empty')

  # ---- the plan --------------------------------------------------------------------------------------------------
  $run = Join-Path $tmp 'run'
  $plan = New-WbQueueSoakPlan -Session1Seconds 300 -Session2Seconds 300 -RunDir $run -Device 'CABLE Output'
  $s = @($plan.sessions)
  Check 'the plan has two sessions: speech first, noise second' ($s.Count -eq 2 -and $s[0].title -eq 'Queue soak speech' -and $s[1].title -eq 'Queue soak noise')
  Check 'session 1 starts at once; session 2 starts after the audio of session 1 and its stop margin' ($s[0].at_s -eq 0 -and $s[1].at_s -ge 300 + 15)
  Check 'each session has its own stop file and the safety ceiling is the audio plus 15 minutes' ($s[0].stop_file -ne $s[1].stop_file -and $s[0].stop_file -like '*STOP-SESSION-1' -and $s[0].duration_s -eq 1200)
  Check 'both sessions use the cable device and the English + Hindi pair' ($s[0].device -eq 'CABLE Output' -and (@($s[0].languages) -join ',') -eq 'en,hi' -and (@($s[1].languages) -join ',') -eq 'en,hi')
  $json = $plan | ConvertTo-Json -Depth 6
  $back = $json | ConvertFrom-Json
  Check 'the plan is valid JSON with sessions and an empty list of actions (the queue is left alone)' (@($back.sessions).Count -eq 2 -and @($back.actions).Count -eq 0)

  # ---- the transcript of session 1 ----------------------------------------------------------------------------------
  function Seg($a, $b) { [pscustomobject]@{ start_ms = $a; end_ms = $b; text = 'x' } }
  $good = @((Seg 2000 9000), (Seg 11000 20000), (Seg 24000 33000), (Seg 35000 44000), (Seg 46000 59000), (Seg 61000 70000), (Seg 72000 78500), (Seg 80000 90000), (Seg 92000 100000), (Seg 101000 118000), (Seg 119500 124000), (Seg 126000 140000), (Seg 142000 150000), (Seg 151000 160000), (Seg 162000 175000), (Seg 176000 190000), (Seg 192000 205000), (Seg 206000 219000), (Seg 221000 236000), (Seg 238000 252000), (Seg 254000 270000), (Seg 272000 286000), (Seg 288000 299000))
  $r = Test-WbQueueSoakSegments -Segments $good -AudioMs 300000
  Check 'a complete transcript passes (23 segments, small natural gaps, both ends covered)' ($r.Ok -and $r.Count -eq 23 -and $r.Overlaps -eq 0 -and $r.DuplicateStarts -eq 0 -and $r.MaxGapMs -le 5000)
  $ov = @($good[0..3]) + @((Seg 40000 50000)) + @($good[4..22])
  $r = Test-WbQueueSoakSegments -Segments $ov -AudioMs 300000
  Check 'a segment that overlaps the one before it fails' (-not $r.Ok -and $r.Overlaps -ge 1 -and ($r.Problems -join ' ') -match 'overlap')
  $dup = @($good) + @((Seg 24000 33000))
  $r = Test-WbQueueSoakSegments -Segments $dup -AudioMs 300000
  Check 'the same segment committed twice (a duplicate start) fails' (-not $r.Ok -and $r.DuplicateStarts -eq 1 -and ($r.Problems -join ' ') -match 'duplicate')
  $gap = @($good | Where-Object { $_.start_ms -lt 60000 -or $_.start_ms -gt 100000 })
  $r = Test-WbQueueSoakSegments -Segments $gap -AudioMs 300000
  Check 'a hole in the middle (a unit that was never committed) fails and names where' (-not $r.Ok -and $r.MaxGapMs -gt 20000 -and ($r.Problems -join ' ') -match 'gap of')
  $short = @($good | Where-Object { $_.end_ms -lt 200000 })
  $r = Test-WbQueueSoakSegments -Segments $short -AudioMs 300000
  Check 'a transcript that stops before the end of the audio fails' (-not $r.Ok -and ($r.Problems -join ' ') -match 'end of the audio is missing')
  $late = @($good | Where-Object { $_.start_ms -gt 30000 })
  $r = Test-WbQueueSoakSegments -Segments $late -AudioMs 300000
  Check 'a transcript that starts late fails' (-not $r.Ok -and ($r.Problems -join ' ') -match 'first segment starts')
  $r = Test-WbQueueSoakSegments -Segments @() -AudioMs 300000
  Check 'no segment at all fails' (-not $r.Ok -and ($r.Problems -join ' ') -match 'no segment')
  $r = Test-WbQueueSoakSegments -Segments $null -AudioMs 300000
  Check 'a missing transcript (null) fails the same way' (-not $r.Ok)
  $r = Test-WbQueueSoakSegments -Segments @((Seg 0 5000), (Seg 6000 400000)) -AudioMs 300000
  Check 'a segment that ends after the audio fails' (-not $r.Ok -and ($r.Problems -join ' ') -match 'after the audio')
  $unordered = @($good[5], $good[0], $good[3], $good[1], $good[2], $good[4]) + @($good[6..22])
  Check 'segments written out of order are judged in audio order' ((Test-WbQueueSoakSegments -Segments $unordered -AudioMs 300000).Ok)

  # ---- the job in the queue report ---------------------------------------------------------------------------------------
  $report = [pscustomobject]@{ queue = [pscustomobject]@{ snapshot = [pscustomobject]@{ jobs = @(
        [pscustomobject]@{ id = 1; session = 'queue-soak-speech'; revision = 'post-1'; kind = 'rolling'; state = 'done'; audio_ms = 300000; cursor_ms = 300000; units_committed = 2; units_total = 2; error = $null },
        [pscustomobject]@{ id = 2; session = 'queue-soak-noise'; revision = 'post-1'; kind = 'rolling'; state = 'running'; audio_ms = 300000; cursor_ms = 120000; units_committed = 0; units_total = 2; error = $null }) } } }
  $job = Get-WbQueueSoakJob -Report $report -Slug 'queue-soak-speech' -Revision 'post-1'
  Check 'the job of session 1 is found by its folder name and revision' ($job -and $job.id -eq 1)
  Check 'no job for another revision' ($null -eq (Get-WbQueueSoakJob -Report $report -Slug 'queue-soak-speech' -Revision 'post-2'))
  Check 'no report, no job' ($null -eq (Get-WbQueueSoakJob -Report $null -Slug 'queue-soak-speech'))
  Check 'a done job at the end of the audio with every unit committed is complete' ((Test-WbQueueSoakJobComplete -Job $job).Ok)
  $notDone = $job.PSObject.Copy(); $notDone.state = 'running'
  Check 'a job still running is not complete' (-not (Test-WbQueueSoakJobComplete -Job $notDone).Ok)
  $shortCursor = $job.PSObject.Copy(); $shortCursor.cursor_ms = 240000
  Check 'a done job whose cursor is short of the audio is not complete' (-not (Test-WbQueueSoakJobComplete -Job $shortCursor).Ok)
  $fewUnits = $job.PSObject.Copy(); $fewUnits.units_committed = 1
  Check 'a done job with an uncommitted unit is not complete' (-not (Test-WbQueueSoakJobComplete -Job $fewUnits).Ok)
  $withErr = $job.PSObject.Copy(); $withErr.error = 'engine failed'
  Check 'a job with an error is not complete and says the error' (-not (Test-WbQueueSoakJobComplete -Job $withErr).Ok -and (Test-WbQueueSoakJobComplete -Job $withErr).Reason -match 'engine failed')
  Check 'no job is not complete' (-not (Test-WbQueueSoakJobComplete -Job $null).Ok)

  # ---- decoding inside the second recording --------------------------------------------------------------------------
  $t0 = [datetime]'2026-11-23T10:00:00'
  $w0 = $t0.AddMinutes(10); $w1 = $t0.AddMinutes(15)
  $u = { param($atMin, $sec) [pscustomobject]@{ At = $t0.AddMinutes($atMin); Seconds = $sec } }
  Check 'a unit that ran wholly inside the window counts in full' ((Get-WbDecodeOverlapSeconds -Units @((& $u 13 120)) -WindowStart $w0 -WindowEnd $w1) -eq 120)
  Check 'a unit that ended before the window counts nothing' ((Get-WbDecodeOverlapSeconds -Units @((& $u 9 120)) -WindowStart $w0 -WindowEnd $w1) -eq 0)
  Check 'a unit that began before and ended inside counts only the part inside' ((Get-WbDecodeOverlapSeconds -Units @((& $u 12 180)) -WindowStart $w0 -WindowEnd $w1) -eq 120)
  Check 'a unit that ended after the window counts only up to its end' ((Get-WbDecodeOverlapSeconds -Units @((& $u 17 300)) -WindowStart $w0 -WindowEnd $w1) -eq 180)
  Check 'two units that ran at once (CPU and graphics) count once' ((Get-WbDecodeOverlapSeconds -Units @((& $u 13 120), (& $u 13 120)) -WindowStart $w0 -WindowEnd $w1) -eq 120)
  Check 'two units one after the other add up' ((Get-WbDecodeOverlapSeconds -Units @((& $u 12 120), (& $u 14 120)) -WindowStart $w0 -WindowEnd $w1) -eq 240)
  Check 'no units, no overlap' ((Get-WbDecodeOverlapSeconds -Units @() -WindowStart $w0 -WindowEnd $w1) -eq 0)

  # ---- the queue log ----------------------------------------------------------------------------------------------------
  $slug = 'queue-soak-speech'
  $tab = "`t"
  $log = @(
    "10:00${tab}DAY: started `"Queue soak speech`" in D:\x",
    "10:06${tab}${slug}: pauses of part 3 found (5 pauses, 0 fallback cuts) in 1.2 s; engine peak 1535 MiB against limit 3106 MiB (measured peak)",
    "10:08${tab}${slug}: stopped. All transcripts ready by 10:13 IST (sessions with several languages: provisional)",
    "10:11${tab}${slug}: unit 0 of post-1 done in 243.1 s on gpu (1 restart(s), engine peak 1535 MiB against limit 3106 MiB)",
    "10:15${tab}${slug}: unit 1 of post-1 done in 98.7 s on cpu (0 restart(s), engine peak 1528 MiB against limit 3000 MiB)",
    "10:16${tab}${slug}: post-1 transcript complete",
    "10:16${tab}other-session: unit 0 of post-1 done in 5.0 s on gpu (engine peak 9999 MiB against limit 1 MiB)")
  $ul = ConvertFrom-WbQueueUnitLine -Line $log[3] -Slug $slug
  Check 'a unit line gives the unit, its seconds and the device' ($ul.Unit -eq 0 -and $ul.Seconds -eq 243.1 -and $ul.Device -eq 'gpu')
  Check 'a unit line of another session is not read' ($null -eq (ConvertFrom-WbQueueUnitLine -Line $log[6] -Slug $slug))
  $li = Get-WbQueueSoakLogInfo -Lines $log -Slug $slug
  Check 'the estimate error is the completion minus the estimate (10:16 against 10:13 = +3 min)' ($li.EstimatedReadyBy -eq '10:13' -and $li.CompletedAt -eq '10:16' -and $li.EstimateErrorMinutes -eq 3)
  Check 'the engine peak is the highest of this session''s lines and the limit the tightest (1535 MiB, 3000 MiB; the other session is ignored)' ($li.EnginePeakMiB -eq 1535 -and $li.EngineLimitMiB -eq 3000)
  $late = Get-WbQueueSoakLogInfo -Lines @("23:55${tab}${slug}: stopped. All transcripts ready by 23:58 IST", "00:03${tab}${slug}: post-1 transcript complete") -Slug $slug
  Check 'the estimate error survives midnight (23:58 to 00:03 = +5 min)' ($late.EstimateErrorMinutes -eq 5)
  $none = Get-WbQueueSoakLogInfo -Lines @() -Slug $slug
  Check 'an empty log gives no estimate and no peak, without an error' ($null -eq $none.EstimateErrorMinutes -and $null -eq $none.EnginePeakMiB)

  # ---- the recording rule per session ----------------------------------------------------------------------------------
  $refPass = [pscustomobject]@{ Verdict = 'PASS'; Reason = 'the app equals both independent references sample for sample' }
  $refFail = [pscustomobject]@{ Verdict = 'FAIL'; Reason = 'the app lost or changed audio' }
  $refInv = [pscustomobject]@{ Verdict = 'INVALID'; Reason = 'the app could not be compared with the WASAPI reference' }
  $rvPass = [pscustomobject]@{ Verdict = 'PASS'; FailReasons = @(); InvalidReasons = @() }
  $rvFail = [pscustomobject]@{ Verdict = 'FAIL'; FailReasons = @('10 counted upstream breaks in 5 minutes (limit 1)'); InvalidReasons = @() }
  $rvInv = [pscustomobject]@{ Verdict = 'INVALID'; FailReasons = @(); InvalidReasons = @('the reference capture is missing') }
  $rec = { param($ref, $rv, $d = 0, $o = 0, $x = 0, $xe = $false, $ro = 0) Get-WbQueueSoakRecordingVerdict -Label 'session 1' -RunVerdict $rv -RefVerdict $ref -Dropped $d -Overrun $o -Xruns $x -XrunExcused $xe -ReferenceOverrun $ro }
  Check 'a clean session passes (equals both references, nothing dropped, no overruns, no xruns)' ((& $rec $refPass $null).Status -eq 'PASS')
  Check 'a clean noise session passes with the Phase 1 verdict too' ((& $rec $refPass $rvPass).Status -eq 'PASS')
  Check 'differing from a reference fails' ((& $rec $refFail $null).Status -eq 'FAIL')
  Check 'a reference that could not be compared makes the session INVALID, not PASS' ((& $rec $refInv $null).Status -eq 'INVALID')
  Check 'no reference verdict at all makes the session INVALID' ((& $rec $null $null).Status -eq 'INVALID')
  Check 'one dropped sample fails' ((& $rec $refPass $rvPass 1).Status -eq 'FAIL')
  Check 'one overrun sample fails' ((& $rec $refPass $rvPass 0 1).Status -eq 'FAIL')
  Check 'an xrun that was not excused fails' ((& $rec $refPass $rvPass 0 0 1 $false).Status -eq 'FAIL')
  Check 'one xrun at the stream start with no loss around it (the Phase 1 excuse) does not fail' ((& $rec $refPass $rvPass 0 0 1 $true).Status -eq 'PASS')
  Check 'too many counted upstream breaks (the Phase 1 rule on the noise session) fail' ((& $rec $refPass $rvFail).Status -eq 'FAIL' -and (& $rec $refPass $rvFail).Fail[0] -match 'upstream breaks')
  Check 'a run the Phase 1 rule calls invalid is INVALID' ((& $rec $refPass $rvInv).Status -eq 'INVALID')
  Check 'a reference capture that lost samples itself is INVALID, not the app''s fault' ((& $rec $refPass $rvPass 0 0 0 $false 5).Status -eq 'INVALID')
  Check 'a failure beats an invalid reason in the same session' ((& $rec $refInv $null 1).Status -eq 'FAIL')

  # ---- the verdict of the gate ------------------------------------------------------------------------------------------
  $segOk = Test-WbQueueSoakSegments -Segments $good -AudioMs 300000
  $segBad = Test-WbQueueSoakSegments -Segments $gap -AudioMs 300000
  $jobOk = Test-WbQueueSoakJobComplete -Job $job
  $jobBad = Test-WbQueueSoakJobComplete -Job $notDone
  $r1 = & $rec $refPass $null
  $r2 = & $rec $refPass $rvPass
  $r1bad = & $rec $refPass $null 0 0 1 $false
  $r2inv = & $rec $refInv $rvPass
  $v = { param($a, $b, $seg, $jc, $ov) Get-WbQueueSoakVerdict -Recording1 $a -Recording2 $b -Segments $seg -JobCheck $jc -OverlapSeconds $ov }
  Check 'everything right: PASS' ((& $v $r1 $r2 $segOk $jobOk 200).Status -eq 'PASS')
  $vp = & $v $r1 $r2 $segOk $jobOk 200
  Check 'a PASS has no failure and no invalid reason, and every check is ticked' (@($vp.Fail).Count -eq 0 -and @($vp.Invalid).Count -eq 0 -and -not ($vp.Checks.Values -contains $false))
  Check 'session 1 recording fails (an unexcused xrun): FAIL' ((& $v $r1bad $r2 $segOk $jobOk 200).Status -eq 'FAIL')
  Check 'session 2 recording fails: FAIL' ((& $v $r1 (& $rec $refFail $rvPass) $segOk $jobOk 200).Status -eq 'FAIL')
  Check 'session 1 not transcribed in full (job not done): FAIL' ((& $v $r1 $r2 $segOk $jobBad 200).Status -eq 'FAIL')
  Check 'session 1 transcript with a hole: FAIL, and the verdict names the gap' (((& $v $r1 $r2 $segBad $jobOk 200).Status -eq 'FAIL') -and ((& $v $r1 $r2 $segBad $jobOk 200).Fail -join ' ') -match 'gap of')
  Check 'session 1 transcript with an overlap: FAIL' ((& $v $r1 $r2 (Test-WbQueueSoakSegments -Segments $ov -AudioMs 300000) $jobOk 200).Status -eq 'FAIL')
  Check 'session 1 transcript with a duplicate: FAIL' ((& $v $r1 $r2 (Test-WbQueueSoakSegments -Segments $dup -AudioMs 300000) $jobOk 200).Status -eq 'FAIL')
  Check 'no queue report: FAIL (the job cannot be judged)' ((Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $segOk -JobCheck $null -OverlapSeconds 200).Status -eq 'FAIL')
  Check 'no transcript read: FAIL' ((Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $null -JobCheck $jobOk -OverlapSeconds 200).Status -eq 'FAIL')
  Check 'the queue did not decode while session 2 recorded (10 s): INVALID, the run measured nothing' ((& $v $r1 $r2 $segOk $jobOk 10).Status -eq 'INVALID' -and ((& $v $r1 $r2 $segOk $jobOk 10).Invalid -join ' ') -match 'did not measure')
  Check 'a disturbed rig (INVALID recording) with everything else right: INVALID' ((& $v $r1 $r2inv $segOk $jobOk 200).Status -eq 'INVALID')
  Check 'a real failure wins over an invalid reason: FAIL' ((& $v $r1 $r2inv $segBad $jobOk 200).Status -eq 'FAIL')
  Check 'a failure wins over the missing overlap: FAIL' ((& $v $r1bad $r2 $segOk $jobOk 5).Status -eq 'FAIL')
  # Information only: no estimate error and no engine peak, however large, can change the verdict (they are not inputs).
  $paramNames = (Get-Command Get-WbQueueSoakVerdict).Parameters.Keys
  Check 'the ready-by estimate error and the engine memory peak are not inputs of the verdict (information only)' (-not ($paramNames | Where-Object { $_ -match 'Estimate|Peak|Memory|ReadyBy' }))
  $info = Get-WbQueueSoakInfoLines -LogInfo $li -OverlapSeconds 200 -Segments $segOk -Job $job
  Check 'the information lines carry the estimate error, the engine peak and the overlap' (($info -join "`n") -match 'error \+3 min' -and ($info -join "`n") -match '1535 MiB' -and ($info -join "`n") -match '200 s')

  $gateTextAll = Get-Content -LiteralPath (Join-Path $repo 'scripts\gates\queue-soak.ps1') -Raw
  # ---- the two modes of a machine (owner, 23 Nov 2026): processing during recording, or waiting -------------------------
  Check 'the mode follows the machine''s own system check (cleared: processing; not cleared or unknown: waiting)' ((Get-WbQueueSoakMode -ProcessWhileRecording $true) -eq 'processing' -and (Get-WbQueueSoakMode -ProcessWhileRecording $false) -eq 'waiting' -and (Get-WbQueueSoakMode -ProcessWhileRecording $null) -eq 'waiting')
  Check 'the evidence text names the mode' ((Get-WbQueueSoakModeText -Mode 'waiting') -match 'did not clear' -and (Get-WbQueueSoakModeText -Mode 'processing') -match 'cleared')
  Check 'waiting mode leaves the app running longer after the recordings: twice the audio of session 1 plus 5 minutes (full 55, quick 15)' ((Get-WbQueueSoakDecodeWaitMinutes -Settings $f) -eq 55 -and (Get-WbQueueSoakDecodeWaitMinutes -Settings $q) -eq 15)
  Check 'processing mode: the hard limit for session 1''s transcript is 30 minutes in quick mode and 90 in full mode' ($q.HardLimitMinutes -eq 30 -and $f.HardLimitMinutes -eq 90 -and $null -eq $q.DecodeWaitMinutes)
  # The wait for session 1 in processing mode (owner, 23 Nov 2026): the app's own estimate plus 10% plus 5 minutes, never past the hard limit.
  $t10 = [datetime]'2026-11-23T10:05:00'
  $sl = ConvertFrom-WbQueueStopLine -Line ("10:05`tqueue-soak-speech: stopped. All transcripts ready by 10:25 IST (from the speed measured so far)") -Slug 'queue-soak-speech'
  Check 'the stop line gives the clock of the stop and the "ready by" time' ($sl.StopClock -eq '10:05' -and $sl.ReadyByClock -eq '10:25')
  Check 'the stop line of another session, or any other line, is not read' ($null -eq (ConvertFrom-WbQueueStopLine -Line ("10:05`tother-session: stopped. All transcripts ready by 10:25 IST") -Slug 'queue-soak-speech') -and $null -eq (ConvertFrom-WbQueueStopLine -Line ("10:05`tqueue-soak-speech: post-1 transcript complete") -Slug 'queue-soak-speech'))
  $wl = Get-WbQueueSoakWaitLimit -StoppedAt $t10 -StopLine $sl -HardLimitMinutes 30
  Check 'estimate 20 min gives a deadline of (20 + 1 clock minute) x 1.1 + 5 = 28.1 minutes after the stop' ([math]::Abs(($wl.Deadline - $t10).TotalMinutes - 28.1) -lt 0.01 -and $wl.EstimateMinutes -eq 20 -and $wl.ReadyByClock -eq '10:25')
  $wlBig = Get-WbQueueSoakWaitLimit -StoppedAt $t10 -StopLine (ConvertFrom-WbQueueStopLine -Line ("10:05`tqueue-soak-speech: stopped. All transcripts ready by 12:30 IST") -Slug 'queue-soak-speech') -HardLimitMinutes 30
  Check 'an estimate past the hard limit is cut at the hard limit (30 min quick), and the basis says so' ([math]::Abs(($wlBig.Deadline - $t10).TotalMinutes - 30) -lt 0.01 -and $wlBig.Basis -match 'past the hard limit')
  $wlFull = Get-WbQueueSoakWaitLimit -StoppedAt $t10 -StopLine (ConvertFrom-WbQueueStopLine -Line ("10:05`tqueue-soak-speech: stopped. All transcripts ready by 12:30 IST") -Slug 'queue-soak-speech') -HardLimitMinutes 90
  Check 'the same estimate in full mode is 145 min, so it is cut at 90' ([math]::Abs(($wlFull.Deadline - $t10).TotalMinutes - 90) -lt 0.01)
  $wlNone = Get-WbQueueSoakWaitLimit -StoppedAt $t10 -StopLine $null -HardLimitMinutes 30
  Check 'no estimate from the app: only the hard limit applies' ([math]::Abs(($wlNone.Deadline - $t10).TotalMinutes - 30) -lt 0.01 -and $null -eq $wlNone.EstimateMinutes)
  $wlMid = Get-WbQueueSoakWaitLimit -StoppedAt $t10 -StopLine (ConvertFrom-WbQueueStopLine -Line ("23:55`tqueue-soak-speech: stopped. All transcripts ready by 00:10 IST") -Slug 'queue-soak-speech') -HardLimitMinutes 90
  Check 'an estimate across midnight is 15 minutes, not negative' ($wlMid.EstimateMinutes -eq 15)
  $vNot = Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $segOk -JobCheck $jobBad -OverlapSeconds 200 -WaitedOut $true -AppReadyClock '10:25'
  Check 'session 1 not finished when the wait ended: FAIL with "not finished; the app estimated HH:MM"' ($vNot.Status -eq 'FAIL' -and ($vNot.Fail -join ' ') -match 'not finished; the app estimated 10:25')
  $vNone = Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $segOk -JobCheck $jobBad -OverlapSeconds 200 -WaitedOut $true
  Check 'the same with no estimate from the app says so' ($vNone.Status -eq 'FAIL' -and ($vNone.Fail -join ' ') -match 'no estimate')
  Check 'finished in time: PASS on this rule, even when the wait flag is set (the estimate error is information only)' ((Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $segOk -JobCheck $jobOk -OverlapSeconds 200 -WaitedOut $true -AppReadyClock '10:25').Status -eq 'PASS')
  Check 'a job not done without the wait having run out keeps the plain reason' (-not (((Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $segOk -JobCheck $jobBad -OverlapSeconds 200).Fail -join ' ') -match 'not finished; the app estimated'))
  Check 'less than 60 s of decode overlap with session 2 is still INVALID in processing mode' ((Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $segOk -JobCheck $jobOk -OverlapSeconds 59).Status -eq 'INVALID')
  Check 'a batch engine is told from its command line; a chunks call apart; other programs and other engine calls are not' ((Get-WbEngineSubcommand -Name 'wb-engine-vulkan-x86_64-pc-windows-msvc.exe' -CommandLine '"C:\p\wb-engine-vulkan.exe" batch --request r.json') -eq 'batch' -and (Get-WbEngineSubcommand -Name 'wb-engine-avx2.exe' -CommandLine 'wb-engine-avx2.exe chunks --file p.wav') -eq 'chunks' -and $null -eq (Get-WbEngineSubcommand -Name 'wb-engine-avx2.exe' -CommandLine 'wb-engine-avx2.exe info') -and $null -eq (Get-WbEngineSubcommand -Name 'notepad.exe' -CommandLine 'notepad.exe batch') -and $null -eq (Get-WbEngineSubcommand -Name 'wb-engine-avx2.exe' -CommandLine $null))
  $w0 = [datetime]'2026-11-23T10:00:00'
  $win = @([pscustomobject]@{ Start = $w0; End = $w0.AddMinutes(5) }, [pscustomobject]@{ Start = $w0.AddMinutes(7); End = $w0.AddMinutes(12) })
  $look = $null
  # A look every 4 s over 0 to 11.3 minutes (two recordings: 10:00 to 10:05 and 10:07 to 10:12). An engine, when given, is
  # alive from $from to $to minutes, created at $created minutes (all relative to 10:00).
  $mk = {
    param($from = $null, $to = $null, $created = $null, $chunkFrom = $null)
    @(0..170 | ForEach-Object {
      $t = $_ * 4 / 60.0
      $e = @()
      if ($null -ne $from -and $t -ge $from -and $t -le $to) { $e = @([pscustomobject]@{ Pid = 77; Created = $w0.AddMinutes($created) }) }
      $c = $(if ($null -ne $chunkFrom -and $t -ge $chunkFrom -and $t -le ($chunkFrom + 0.2)) { 1 } else { 0 })
      [pscustomobject]@{ At = $w0.AddMinutes($t); Engines = @($e); Chunks = $c }
    })
  }
  $obsClean = Get-WbBatchDuringRecording -Samples (& $mk) -Windows $win
  Check 'looks inside both recordings are counted, those between them are not' ($obsClean.InWindowSamples -gt 60 -and $obsClean.Violations -eq 0 -and $obsClean.InWindowSamples -lt 171)
  Check 'with no engine running at the start of a recording the evidence says so' (-not $obsClean.Stops[1].HadRunningEngine -and $null -eq $obsClean.Stops[1].StopLowerSeconds)
  # The queue code stops a running unit at once when a recording starts (queue/mod.rs:543-550, supervisor run.rs:268-269); the rule
  # is: gone within 10 s, nothing new started inside a recording.
  $obsFast = Get-WbBatchDuringRecording -Samples (& $mk 6.2 7.1 6.2) -Windows $win
  Check 'an engine running at the start of recording 2 that is gone 8 s later is fine (limit 10 s), and its stop time is reported (4 to 8 s)' ($obsFast.Violations -eq 0 -and $obsFast.Stops[1].HadRunningEngine -and $obsFast.Stops[1].StopLowerSeconds -eq 4 -and $obsFast.Stops[1].StopUpperSeconds -eq 8 -and -not $obsFast.Stops[1].StillAliveAtEnd)
  $obsSlow = Get-WbBatchDuringRecording -Samples (& $mk 6.2 7.5 6.2) -Windows $win
  Check 'an engine still alive 28 s after the start is a violation, and the first one is named (12 s)' ($obsSlow.Violations -eq 5 -and $obsSlow.LateSeconds -eq 10 -and $obsSlow.FirstViolation -match 'still alive 12 s later')
  $obsStuck = Get-WbBatchDuringRecording -Samples (& $mk 6.2 12 6.2) -Windows $win
  Check 'an engine that never went is a violation and the evidence says it was alive at the end' ($obsStuck.Violations -gt 0 -and $obsStuck.Stops[1].StillAliveAtEnd)
  $obsNew = Get-WbBatchDuringRecording -Samples (& $mk 8 8.5 8) -Windows $win
  Check 'a batch engine that started inside a recording is a violation at once, counted once however many looks saw it' ($obsNew.NewEngines -eq 1 -and $obsNew.Violations -eq 1 -and $obsNew.FirstViolation -match 'started at 10:08:00')
  $obsNew1 = Get-WbBatchDuringRecording -Samples (& $mk 0.5 1 0.5) -Windows $win
  Check 'a batch engine that started inside the first recording is a violation too' ($obsNew1.NewEngines -eq 1 -and $obsNew1.Violations -eq 1)
  $obsBetween = Get-WbBatchDuringRecording -Samples (& $mk 5.5 6.5 5.5) -Windows $win
  Check 'a batch engine that lived between the two recordings and was gone before the next began is allowed (the queue may work then)' ($obsBetween.Violations -eq 0)
  $obsChunk = Get-WbBatchDuringRecording -Samples (& $mk $null $null $null 2) -Windows $win
  Check 'a chunks call inside a recording is counted apart and is not a batch engine' ($obsChunk.ChunkSamples -gt 0 -and $obsChunk.Violations -eq 0)
  Check 'no looks at all inside any recording is zero looks, not a pass' ((Get-WbBatchDuringRecording -Samples @() -Windows $win).InWindowSamples -eq 0)
  Check 'a different grace period moves the limit: a limit of 3 s fails the engine seen 4 s after the start' ((Get-WbBatchDuringRecording -Samples (& $mk 6.2 7.1 6.2) -Windows $win -GraceSeconds 3).Violations -gt 0)
  $vw = { param($a, $b, $seg, $jc, $obs) Get-WbQueueSoakVerdict -Recording1 $a -Recording2 $b -Segments $seg -JobCheck $jc -OverlapSeconds 0 -Mode 'waiting' -BatchObservation $obs }
  Check 'waiting mode, no batch engine while recording, session 1 complete, both recordings clean: PASS (no overlap needed)' ((& $vw $r1 $r2 $segOk $jobOk $obsClean).Status -eq 'PASS')
  Check 'waiting mode, a unit stopped within 10 s of the start of session 2: PASS' ((& $vw $r1 $r2 $segOk $jobOk $obsFast).Status -eq 'PASS')
  Check 'the same inputs in processing mode are INVALID: there the queue had to decode during session 2' ((& $v $r1 $r2 $segOk $jobOk 0).Status -eq 'INVALID')
  $vwSlow = & $vw $r1 $r2 $segOk $jobOk $obsSlow
  Check 'waiting mode, an engine alive more than 10 s after a recording began: FAIL, the verdict names the first look' ($vwSlow.Status -eq 'FAIL' -and ($vwSlow.Fail -join ' ') -match 'waiting rule' -and ($vwSlow.Fail -join ' ') -match 'still alive 12 s later')
  $vwNew = & $vw $r1 $r2 $segOk $jobOk $obsNew
  Check 'waiting mode, a new batch engine during a recording: FAIL' ($vwNew.Status -eq 'FAIL' -and ($vwNew.Fail -join ' ') -match 'started at 10:08:00')
  Check 'waiting mode, session 1 not committed in full after session 2: FAIL' ((& $vw $r1 $r2 $segOk $jobBad $obsClean).Status -eq 'FAIL')
  Check 'waiting mode, a hole in the transcript: FAIL' ((& $vw $r1 $r2 $segBad $jobOk $obsClean).Status -eq 'FAIL')
  Check 'waiting mode, a duplicate in the transcript: FAIL' ((& $vw $r1 $r2 (Test-WbQueueSoakSegments -Segments $dup -AudioMs 300000) $jobOk $obsClean).Status -eq 'FAIL')
  Check 'waiting mode, session 2 fails the Phase 1 rule: FAIL' ((& $vw $r1 (& $rec $refFail $rvFail) $segOk $jobOk $obsClean).Status -eq 'FAIL')
  Check 'waiting mode, session 1 recording fails: FAIL' ((& $vw $r1bad $r2 $segOk $jobOk $obsClean).Status -eq 'FAIL')
  Check 'waiting mode, too few looks inside the recordings: INVALID, the rule was not observed' ((& $vw $r1 $r2 $segOk $jobOk (Get-WbBatchDuringRecording -Samples @([pscustomobject]@{ At = $w0.AddMinutes(1); Engines = @(); Chunks = 0 }, [pscustomobject]@{ At = $w0.AddMinutes(2); Engines = @(); Chunks = 0 }) -Windows $win)).Status -eq 'INVALID')
  Check 'waiting mode with no observation at all: INVALID' ((& $vw $r1 $r2 $segOk $jobOk $null).Status -eq 'INVALID')
  Check 'waiting mode: a new batch engine fails even when too few looks were taken' ((& $vw $r1 $r2 $segOk $jobOk (Get-WbBatchDuringRecording -Samples @([pscustomobject]@{ At = $w0.AddMinutes(1); Engines = @([pscustomobject]@{ Pid = 5; Created = $w0.AddMinutes(0.9) }); Chunks = 0 }) -Windows $win)).Status -eq 'FAIL')
  Check 'waiting mode, session 1 not finished when the wait ran out: FAIL with the app''s estimate' ((& $vw $r1 $r2 $segOk $jobBad $obsClean).Status -eq 'FAIL' -and ((Get-WbQueueSoakVerdict -Recording1 $r1 -Recording2 $r2 -Segments $segOk -JobCheck $jobBad -Mode 'waiting' -BatchObservation $obsClean -WaitedOut $true -AppReadyClock '10:40').Fail -join ' ') -match 'not finished; the app estimated 10:40')
  $infoW = Get-WbQueueSoakInfoLines -LogInfo $li -OverlapSeconds 0 -Segments $segOk -Job $job -Mode 'waiting' -BatchObservation $obsFast
  Check 'the information lines carry the mode, the looks at the engine processes and the measured stop time' (($infoW -join "`n") -match 'machine mode: waiting' -and ($infoW -join "`n") -match 'looks at the engine processes inside the recordings' -and ($infoW -join "`n") -match 'stopped between 4\.0 and 8\.0 s after the start')
  Check 'the gate script has no NOT APPLICABLE exit any more and writes the machine mode into the evidence' ($gateTextAll -notmatch 'NOT APPLICABLE' -and $gateTextAll -match "machine mode: ' \+ \(Get-WbQueueSoakModeText")
  Check 'the gate script looks at the engine processes every 2 s only in waiting mode, with each engine''s process id and start time' ($gateTextAll -match "\`$mode -eq 'waiting' -and \(\(Get-Date\) - \`$lastLook\)\.TotalSeconds -ge 2" -and $gateTextAll -match 'CreationDate')
  Check 'the gate script waits for session 1 by the app''s own estimate in processing mode and no longer by a fixed time' ($gateTextAll -match 'Get-WbQueueSoakWaitLimit' -and $gateTextAll -notmatch '\$settings\.DecodeWaitMinutes')
  Check 'the wrapper no longer answers INFO for a machine that did not clear processing' ((Get-Content -LiteralPath (Join-Path $repo 'scripts\gate-pack\gates\queue-soak.ps1') -Raw) -notmatch 'not applicable')

  # ---- the runner: quick-check before queue-soak, always ----------------------------------------------------------------
  $mf = Read-PackManifest -Path (Join-Path $repo 'scripts\gate-pack\gates.manifest.json')
  $g = @($mf.gates | Where-Object { $_.id -eq 'queue-soak' })
  Check 'the registry has queue-soak: P1, unattended, after quick-check, one ledger row that matches GATES.md' ($g.Count -eq 1 -and $g[0].priority -eq 'P1' -and -not $g[0].interactive -and @($g[0].depends_on) -contains 'quick-check' -and @($g[0].ledger).Count -eq 1)
  Check 'the registry states about 25 minutes quick and about an hour full' ($g[0].quick.minutes -le 25 -and $g[0].est_minutes.low -ge 60 -and $g[0].est_minutes.low -le 90)
  Check 'the registry is valid and the ledger row matches exactly one row of docs/GATES.md' (@(Get-PackManifestProblems -Manifest $mf).Count -eq 0 -and @(@(Test-PackLedgerMatches -Manifest $mf -GatesMdPath (Join-Path $repo 'docs\GATES.md')) | Where-Object { $_.Gate -eq 'queue-soak' -and $_.Count -ne 1 }).Count -eq 0)
  $facts = @{ interface = $true; ac = $true; idle = $true }
  $st = New-PackState -Role low
  $sel = Select-PackNextGate -Manifest $mf -State $st -Facts $facts -Only @('queue-soak')
  Check 'asked for by name before the quick check has run, queue-soak is not started: it waits for quick-check' ($null -eq $sel.Next -and (@($sel.Waiting | Where-Object { $_.Id -eq 'queue-soak' } | ForEach-Object { $_.Reasons }) -join ' ') -match "waits for gate 'quick-check'")
  Set-PackGateStatus -State $st -Id 'engines-info' -Fields @{ status = 'INFO' }
  $sel = Select-PackNextGate -Manifest $mf -State $st -Facts $facts -Only @('queue-soak', 'quick-check')
  Check 'with both selected and nothing run, the quick check goes first' ($sel.Next.id -eq 'quick-check')
  Set-PackGateStatus -State $st -Id 'quick-check' -Fields @{ status = 'INFO' }
  $sel = Select-PackNextGate -Manifest $mf -State $st -Facts $facts -Only @('queue-soak')
  Check 'once the quick check has a result, queue-soak is the next gate' ($sel.Next.id -eq 'queue-soak')
  $st2 = New-PackState -Role low
  Set-PackGateStatus -State $st2 -Id 'engines-info' -Fields @{ status = 'PASS' }
  $seq = @()
  for ($i = 0; $i -lt 6; $i++) {
    $nx = Select-PackNextGate -Manifest $mf -State $st2 -Facts $facts -Skip @('speed-table-igpu', 'load-cold', 'load-settled', 'hashing-1', 'network-off', 'panic', 'webview', 'kill')
    if ($null -eq $nx.Next) { break }
    $seq += [string]$nx.Next.id
    Set-PackGateStatus -State $st2 -Id ([string]$nx.Next.id) -Fields @{ status = 'PASS' }
  }
  Check ('in a whole run the quick check comes before queue-soak (order used: ' + ($seq -join ', ') + ')') ($seq.IndexOf('quick-check') -ge 0 -and $seq.IndexOf('queue-soak') -gt $seq.IndexOf('quick-check'))
  Check 'the pack builder copies the gate script and the README tells the person about it' ((Get-Content -LiteralPath (Join-Path $repo 'scripts\build-gate-pack.ps1') -Raw) -match "'queue-soak\.ps1'" -and (Get-Content -LiteralPath (Join-Path $repo 'scripts\gate-pack\README.txt') -Raw) -match 'queue check')

  # ---- the app gets a data folder of its own, with the quick check's capability.json -----------------------------------------
  $gateText = Get-Content -LiteralPath (Join-Path $repo 'scripts\gates\queue-soak.ps1') -Raw
  $iSet = $gateText.IndexOf('Set-WbGateDataDir')
  $iView = $gateText.IndexOf("& `$startApp 'view'")
  $iDay = $gateText.IndexOf("& `$startApp 'day'")
  Check 'queue-soak.ps1 sets WB_GATE_DATA_DIR (Set-WbGateDataDir) before it starts the app for any purpose' ($iSet -gt 0 -and $iView -gt $iSet -and $iDay -gt $iSet)
  Check 'the data folder is inside the run folder, not the real one' ($gateText -match "Set-WbGateDataDir -Dir \(Join-Path \`$run 'appdata'\)")
  Check 'the capability.json of this run''s quick check is copied into that data folder' ($gateText -match "Copy-Item -LiteralPath \`$CapabilityFile -Destination \(Join-Path \`$dataDir 'capability\.json'\)")
  Check 'the gate stops with NEEDS A SYSTEM CHECK (exit 3) when the file is missing, before the rig or the app' ($gateText.IndexOf('NEEDS A SYSTEM CHECK') -gt 0 -and $gateText.IndexOf('NEEDS A SYSTEM CHECK') -lt $gateText.IndexOf('Assert-WbNoResidentAudio'))
  $wrap = Get-Content -LiteralPath (Join-Path $repo 'scripts\gate-pack\gates\queue-soak.ps1') -Raw
  Check 'the wrapper reads the capability.json of the quick-check gate folder and answers PENDING before it calls the gate script' ($wrap -match "'quick-check'" -and $wrap.IndexOf("-Status 'PENDING'") -gt 0 -and $wrap.IndexOf("-Status 'PENDING'") -lt $wrap.IndexOf('Invoke-PackLegacyGate'))
  $asciiOnly = { param($t) -not ($t.ToCharArray() | Where-Object { [int]$_ -gt 126 }) }
  Check 'the module, both scripts and this test are ASCII only' ((& $asciiOnly $gateText) -and (& $asciiOnly $wrap) -and (& $asciiOnly ([IO.File]::ReadAllText((Join-Path $repo 'scripts\lib\QueueSoak.psm1')))))
  Check 'the speech of session 1 is the public FLEURS clips (bench-en.wav, bench-hi.wav), no private audio' ($gateText -match "bench-en\.wav" -and $gateText -match "bench-hi\.wav" -and $gateText -match 'CC BY 4\.0')
  Check 'no night pass runs during the gate (WB_GATE_NIGHT_WAIT_S is long) and no hybrid is forced' ($gateText -match "WB_GATE_NIGHT_WAIT_S = '7200'" -and $gateText -match 'Remove-Item "Env:\\\$v"' -and $gateText -match 'WB_GATE_FORCE_HYBRID')
}
finally {
  Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($fails) { Write-Host "$fails check(s) failed"; exit 1 }
Write-Host 'QueueSoak tests passed'
