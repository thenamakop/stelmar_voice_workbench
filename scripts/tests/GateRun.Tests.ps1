# Tests for the gate-evaluation helpers in scripts/lib/GateRun.psm1 (no gate is run).
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/GateRun.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\GateRun.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Disc([double]$SourceMs, [double]$JumpMs) {
  [pscustomobject]@{ source_ms = $SourceMs; source_jump_ms = $JumpMs; source_jump_frames = [int64]($JumpMs * 48) }
}
function Range([double]$SourceMs, [double]$Ms) { [pscustomobject]@{ source_ms = $SourceMs; ms = $Ms } }

# Unexplained unmatched audio: only the overlap with +-100 ms of a break is explained.
$app = [pscustomobject]@{ unmatched_ranges = @((Range 1000 64)) }
Check 'a short stretch at a break is explained' ((Get-WbUnexplainedUnmatchedMs -App $app -Near @((Disc 1020 -10))) -eq 0)
$app = [pscustomobject]@{ unmatched_ranges = @((Range 1000 2000)) }
Check 'a 2 s stretch starting at a break is mostly unexplained' ((Get-WbUnexplainedUnmatchedMs -App $app -Near @((Disc 1000 300))) -eq 1900)
Check 'with no breaks everything is unexplained' ((Get-WbUnexplainedUnmatchedMs -App $app -Near @()) -eq 2000)
$app = [pscustomobject]@{ unmatched_ranges = @((Range 1000 400)) }
Check 'overlapping windows are not counted twice' ((Get-WbUnexplainedUnmatchedMs -App $app -Near @((Disc 1050 5), (Disc 1100 5))) -eq 200)
$app = [pscustomobject]@{ unmatched_ranges = @() }
Check 'no ranges, nothing unexplained' ((Get-WbUnexplainedUnmatchedMs -App $app -Near @((Disc 500 -10))) -eq 0)

# Discontinuity split: at most one short start-up repeat shared with the reference is excused.
$ref = [pscustomobject]@{ discontinuities = @((Disc 400 -12), (Disc 2600 -12), (Disc 50000 -11), (Disc 60000 300)) }
$app = [pscustomobject]@{ discontinuities = @((Disc 410 -12), (Disc 2600 -12), (Disc 50000 -11), (Disc 60000 300), (Disc 70000 -10)) }
$split = Split-WbDiscontinuities -App $app -Reference $ref
Check 'one start-up repeat excused' ($split.Excused.Count -eq 1 -and [double]$split.Excused[0].source_ms -eq 410)
Check 'a second start-up repeat, a late repeat and a forward jump are capture-chain breaks' ($split.Chain.Count -eq 3)
Check 'a break only the app has is its own' ($split.Own.Count -eq 1 -and [double]$split.Own[0].source_ms -eq 70000)


# ---- The overnight load run's crash: a null analysis under strict mode ------------------------
# The analyser found no signal, so analysis.json was never written and $r.Analysis was $null.
# Reading a property of $null throws under strict mode (PropertyNotFoundStrict), which ended the
# script with no verdict. Test-WbGateData names the missing pieces so the gate can report them.
$r = [pscustomobject]@{ Analysis = $null; Decode = $null; Report = [pscustomobject]@{ stopped = 1 } }
$threw = $false
try { Set-StrictMode -Version 2; $null = $r.Analysis.discontinuities } catch { $threw = ($_.FullyQualifiedErrorId -like 'PropertyNotFoundStrict*') } finally { Set-StrictMode -Off }
Check 'reading a property of a null analysis throws under strict mode (the load run crash)' $threw
Check 'Test-WbGateData names the null members' ((@(Test-WbGateData -Result $r -Required 'Analysis', 'Decode', 'Report') -join ',') -eq 'Analysis,Decode')
Check 'Test-WbGateData names members that are absent altogether' ((@(Test-WbGateData -Result ([pscustomobject]@{}) -Required 'Analysis') -join ',') -eq 'Analysis')
Check 'Test-WbGateData is empty when everything is there' (@(Test-WbGateData -Result $r -Required 'Report').Count -eq 0)

# ---- Invoke-WbGate: hermetic (temp lock files, no RAM minimum unless the test sets one) ---------
$tmp = Join-Path $env:TEMP "wb-gaterun-tests-$PID"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$gateLock = Join-Path $tmp 'gate.lock'
$busyLock = Join-Path $tmp 'busy.lock'
$overlay = @(Get-WbOverlayTools)
if ($overlay.Count -gt 0) {
  Write-Host "skip Invoke-WbGate tests: overlay tools are running ($($overlay -join ', '))"
} else {
  $module = Get-Module GateRun
  # An exception from the body: the lock is removed, and the full exception and stack are kept.
  $threwGate = $false
  try { Invoke-WbGate -Name 't-throw' -LockPath $gateLock -TrackBBusyPath $busyLock -MinFreeRamGB 0 -Body { throw [System.ArgumentException]::new('boom from the body') } } catch { $threwGate = $true }
  $err = & $module { $script:Current.Error }
  Check 'an exception from the body still ends the gate with the lock removed' ($threwGate -and -not (Test-Path $gateLock))
  Check 'the exception type, message and script stack are kept for the evidence' ($err -match 'System\.ArgumentException' -and $err -match 'boom from the body' -and $err -match 'at <ScriptBlock>')

  # Track B's busy lock held longer than the limit: stop and ask, take no lock.
  Set-Content -LiteralPath $busyLock -Value 'step: test' -Encoding ascii
  $msg = ''
  try { Invoke-WbGate -Name 't-wait' -LockPath $gateLock -TrackBBusyPath $busyLock -TrackBWaitMinutes 0 -MinFreeRamGB 0 -Body { 'should not run' } } catch { $msg = $_.Exception.Message }
  Check 'a busy lock held past the limit stops the gate and asks the user' ($msg -match 'Ask the user before running gate t-wait' -and -not (Test-Path $gateLock))

  # Track B's lock that clears while waiting: the gate waits, then runs, and says it waited.
  $remover = Start-Job -ScriptBlock { param($f) Start-Sleep -Seconds 2; Remove-Item -LiteralPath $f -Force } -ArgumentList $busyLock
  $out = Invoke-WbGate -Name 't-clears' -LockPath $gateLock -TrackBBusyPath $busyLock -TrackBPollSeconds 1 -MinFreeRamGB 0 -Body { 'ran' }
  [void](Wait-Job $remover -Timeout 10); Remove-Job $remover -Force
  $cur = & $module { $script:Current }
  Check 'a busy lock that clears lets the gate run afterwards' ((@($out)[-1] -eq 'ran') -and $cur.TrackBWait -match '^waited [0-9.]+ min for .* to clear$')
  Check 'free RAM is recorded at start and at end' ($cur.FreeRamStart -gt 0 -and $cur.FreeRamEnd -gt 0)

  # Not enough free RAM: refuse.
  $msg = ''
  try { Invoke-WbGate -Name 't-ram' -LockPath $gateLock -TrackBBusyPath $busyLock -MinFreeRamGB 100000 -Body { 'should not run' } } catch { $msg = $_.Exception.Message }
  Check 'less free RAM than the minimum refuses to start' ($msg -match 'GB of RAM free; gate t-ram needs at least 100000 GB' -and -not (Test-Path $gateLock))
}

# ---- Crash evidence: FAIL verdict, reason and details, redacted ---------------------------------
$evidence = Join-Path $tmp 'crash-evidence.txt'
Write-WbGateCrashEvidence -Path $evidence -Reason 'the test gate stopped on an error: boom' -Detail ("System.ArgumentException: boom`r`nat <ScriptBlock>, $env:USERPROFILE\secret\run.ps1: line 3") -Machine 'TestMachine' -Profile 'emu-low'
$text = Get-Content -LiteralPath $evidence -Raw
Check 'crash evidence says FAIL, why, and the details' ($text -match 'verdict: FAIL \(the gate did not complete\)' -and $text -match 'reason: the test gate stopped on an error: boom' -and $text -match 'System\.ArgumentException: boom')
Check 'crash evidence is redacted (no user profile path)' ($text -notmatch [regex]::Escape($env:USERPROFILE) -and $text -match '%USERPROFILE%')
Check 'crash evidence names the machine and profile' ($text -match 'machine: TestMachine' -and $text -match 'emulation profile: emu-low')

# ---- The analyser's stderr is captured, and no report is left when it found no signal -----------
# where.exe stands in for wb-testbench: it rejects the arguments with a message on stderr and exits non-zero.
$report = Join-Path $tmp 'analysis.json'
$stderrText = Invoke-WbAnalyse -Testbench "$env:SystemRoot\System32\where.exe" -Session $tmp -Source $tmp -Report $report
Check 'the analyser failure text is captured' ($stderrText.Length -gt 0)
Check 'no report is left behind' (-not (Test-Path $report))

# ---- The cable as a default playback device (system sounds would reach the test signal) ---------
function New-FakeTestbench([string]$Name, [string]$Json, [int]$ExitCode = 0) {
  $f = Join-Path $tmp "$Name.cmd"
  Set-Content -LiteralPath $f -Encoding ascii -Value @('@echo off', "echo $Json", "exit /b $ExitCode")
  $f
}
$fake = New-FakeTestbench 'fake-cable-default' '{"default_output":"x","default_roles":{"console":"CABLE Input (VB-Audio Virtual Cable)","multimedia":"Speakers (Realtek(R) Audio)","communications":"CABLE Input (VB-Audio Virtual Cable)"}}'
Check 'roles whose default playback device is the cable are named' ((@(Get-WbCableDefaultRoles -Testbench $fake -Cable 'CABLE Input') -join ',') -eq 'console,communications')
$fake = New-FakeTestbench 'fake-speakers' '{"default_roles":{"console":"Speakers (Realtek(R) Audio)","multimedia":"Speakers (Realtek(R) Audio)","communications":"Speakers (Realtek(R) Audio)"}}'
Check 'no role is named when the default is the speakers' (@(Get-WbCableDefaultRoles -Testbench $fake -Cable 'CABLE Input').Count -eq 0)
$fake = New-FakeTestbench 'fake-null-roles' '{"default_output":"x","default_roles":null}'
Check 'a testbench that could not read the roles gives none (not a crash)' (@(Get-WbCableDefaultRoles -Testbench $fake -Cable 'CABLE Input').Count -eq 0)
$fake = New-FakeTestbench 'fake-fails' 'broken' 1
Check 'a failing testbench gives none (not a crash)' (@(Get-WbCableDefaultRoles -Testbench $fake -Cable 'CABLE Input').Count -eq 0)

# ---- The kill gate's crash: @($list) on a List[object] of objects (Windows PowerShell 5.1) ------
$rounds = New-Object System.Collections.Generic.List[object]
$rounds.Add([pscustomobject]@{ Kill = 1; KillAt = [DateTimeOffset]::Now; Parts = 'a' })
$rounds.Add([pscustomobject]@{ Kill = 2; KillAt = [DateTimeOffset]::Now; Parts = 'b' })
$atThrows = $false
try { $null = [pscustomobject]@{ Rounds = @($rounds) } } catch { $atThrows = ($_.Exception -is [System.ArgumentException]) }
Write-Host "info: @(List[object]) throws 'Argument types do not match' on this PowerShell: $atThrows"
$obj = [pscustomobject]@{ Rounds = @(ConvertTo-WbArray $rounds) }
Check 'ConvertTo-WbArray builds the result object the kill gate needs' ($obj.Rounds.Count -eq 2 -and $obj.Rounds[1].Kill -eq 2)
$one = New-Object System.Collections.Generic.List[object]; $one.Add([pscustomobject]@{ Kill = 1 })
Check 'a one-element list stays an array' (@(ConvertTo-WbArray $one).Count -eq 1 -and (@(ConvertTo-WbArray $one) -is [array]))
$none = New-Object System.Collections.Generic.List[string]
Check 'an empty list gives an empty array' (@(ConvertTo-WbArray $none).Count -eq 0)
Check 'a null list gives an empty array' (@(ConvertTo-WbArray $null).Count -eq 0)
$gateDir = Join-Path $here '..\gates'
$bad = @(Get-ChildItem $gateDir -Filter *.ps1 | Where-Object { (Get-Content $_.FullName -Raw) -match '@\(\$(rounds|interference|progress)\)' } | ForEach-Object Name)
Check "no gate script uses @() on its Lists (use ConvertTo-WbArray): $($bad -join ', ')" ($bad.Count -eq 0)

# ---- The driver-xrun rule: one per run within 2 s of a stream start, only with zero loss -------
function New-XrunSession([string]$Name, $Events) {
  $dir = Join-Path $tmp $Name
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $all = @($Events | ForEach-Object { [pscustomobject]@{ at = '2026-11-09T09:00:00+05:30'; audio_ms = $_[0]; kind = $_[1]; detail = $_[2] } })
  ([pscustomobject]@{ events = $all } | ConvertTo-Json -Depth 5) | Set-Content -LiteralPath (Join-Path $dir 'session.json') -Encoding utf8
  $dir
}
$start0 = @(0, 'audio_started', 'CABLE Output 48000 Hz 2 ch')
$start1 = @(120000, 'audio_started', 'CABLE Output 48000 Hz 2 ch')
$oneXrun = 'overrun_samples+0 xruns+1 dropped_samples+0 (totals 0, 1, 0)'

$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-none' @($start0))
Check 'no xruns passes' ($v.Pass -and $v.TotalXruns -eq 0)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-one' @($start0, @(970, 'gap', $oneXrun)))
Check 'one xrun in the first second of a stream passes and is excused' ($v.Pass -and $v.TotalXruns -eq 1 -and @($v.Excused).Count -eq 1 -and [math]::Round($v.Excused[0].OffsetMs) -eq 970)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-restarts' @($start0, @(970, 'gap', $oneXrun), $start1, @(120970, 'gap', $oneXrun)))
Check 'one xrun after each of two stream starts (a restart) passes' ($v.Pass -and $v.TotalXruns -eq 2 -and @($v.Excused).Count -eq 2)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-at-limit' @($start0, @(2000, 'gap', $oneXrun)))
Check 'an xrun exactly at the 2 s limit passes' ($v.Pass)

$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-two-in-one' @($start0, @(970, 'gap', 'overrun_samples+0 xruns+2 dropped_samples+0 (totals 0, 2, 0)')))
Check 'two xruns in one run fail' (-not $v.Pass -and ($v.Problems -join ' ') -match 'more than one')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-two-events' @($start0, @(970, 'gap', $oneXrun), @(1500, 'gap', $oneXrun)))
Check 'a second xrun in the same run fails even inside the window' (-not $v.Pass -and ($v.Problems -join ' ') -match 'more than one')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-late' @($start0, @(3500, 'gap', $oneXrun)))
Check 'an xrun after 2 s fails' (-not $v.Pass -and ($v.Problems -join ' ') -match '3500 ms after the stream started')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-late-restart' @($start0, @(970, 'gap', $oneXrun), $start1, @(123000, 'gap', $oneXrun)))
Check 'the window is measured from each run''s own start' (-not $v.Pass -and ($v.Problems -join ' ') -match '3000 ms after')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-loss' @($start0, @(970, 'gap', $oneXrun))) -LossFree $false
Check 'an xrun with loss or duplication in the recording fails' (-not $v.Pass -and ($v.Problems -join ' ') -match 'loss or duplication')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-overrun' @($start0, @(970, 'gap', 'overrun_samples+480 xruns+0 dropped_samples+0 (totals 480, 0, 0)')))
Check 'a ring overrun fails whatever the xruns' (-not $v.Pass -and ($v.Problems -join ' ') -match '480 overrun')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-dropped' @($start0, @(970, 'gap', 'overrun_samples+0 xruns+0 dropped_samples+960 (totals 0, 0, 960)')))
Check 'dropped samples fail' (-not $v.Pass -and ($v.Problems -join ' ') -match '960 dropped')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-garbled' @($start0, @(970, 'gap', 'something else')))
Check 'a gap event that cannot be read fails instead of being skipped' (-not $v.Pass)

# The `cause=xrun` events the recorder has written since 10 Nov 2026 (callback count, audio position, QPC), next to the
# counter events. The first fixture is the real session of the Night 1 kill run (6 app starts, 11 Nov 2026), events
# copied unchanged; that gate failed only because this reader rejected the new events.
function New-CauseXrun([int]$N, [int]$Callback, [double]$AudioMs) { "cause=xrun n=$N callback=$Callback frames=0 audio_ms=$([int64]$AudioMs) qpc=820239985771 qpc_hz=10000000" }
$realDir = Join-Path $tmp 'x-real-kill'
New-Item -ItemType Directory -Force -Path $realDir | Out-Null
Copy-Item -LiteralPath (Join-Path $here 'fixtures\kill-session-xruns-20261111.json') -Destination (Join-Path $realDir 'session.json')
$v = Get-WbXrunVerdict -SessionDir $realDir
Check 'the real kill session (6 starts, cause=xrun plus counter events) passes with 6 excused and no problems' ($v.Pass -and $v.TotalXruns -eq 6 -and @($v.Excused).Count -eq 6 -and @($v.Problems).Count -eq 0)
Check 'the real kill session: every excused xrun is exact, at callback 0, offset 0 ms, with its QPC tick' (@($v.Excused | Where-Object { $_.Exact -and $_.Callback -eq 0 -and $_.OffsetMs -eq 0 -and $_.Qpc -gt 0 }).Count -eq 6)
$v = Get-WbXrunVerdict -SessionDir $realDir -LossFree $false
Check 'the real kill session fails when the recording shows loss' (-not $v.Pass -and @($v.Problems).Count -eq 6)

$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-cause-only' @($start0, @(0, 'gap', (New-CauseXrun 1 0 0))))
Check 'a cause=xrun event alone counts one xrun, exact, at the stream start' ($v.Pass -and $v.TotalXruns -eq 1 -and $v.Excused[0].Exact -and $v.Excused[0].OffsetMs -eq 0)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-cause-and-counter' @($start0, @(0, 'gap', (New-CauseXrun 1 0 0)), @(970, 'gap', $oneXrun)))
Check 'a cause=xrun event and the counter event of the same xrun count once, not twice' ($v.Pass -and $v.TotalXruns -eq 1)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-cause-late' @($start0, @(5000, 'gap', (New-CauseXrun 1 400 5000))))
Check 'a cause=xrun event after 2 s fails, with its exact offset' (-not $v.Pass -and ($v.Problems -join ' ') -match '5000 ms after the stream started')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-cause-two' @($start0, @(0, 'gap', (New-CauseXrun 1 0 0)), @(900, 'gap', (New-CauseXrun 2 90 900)), @(970, 'gap', 'overrun_samples+0 xruns+2 dropped_samples+0 (totals 0, 2, 0)')))
Check 'two cause=xrun events in one run fail' (-not $v.Pass -and $v.TotalXruns -eq 2 -and ($v.Problems -join ' ') -match 'more than one')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-cause-capped' @($start0, @(0, 'gap', (New-CauseXrun 1 0 0)), @(970, 'gap', 'overrun_samples+0 xruns+2 dropped_samples+0 (totals 0, 2, 0)')))
Check 'xruns the counters saw beyond the cause=xrun events (the event cap) still count and fail' (-not $v.Pass -and $v.TotalXruns -eq 2)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-cause-overrun' @($start0, @(0, 'gap', (New-CauseXrun 1 0 0)), @(970, 'gap', 'overrun_samples+480 xruns+1 dropped_samples+0 (totals 480, 1, 0)')))
Check 'with cause=xrun events present the counter events still fail on overrun samples' (-not $v.Pass -and ($v.Problems -join ' ') -match '480 overrun')
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-cause-restarts' @($start0, @(0, 'gap', (New-CauseXrun 1 0 0)), @(970, 'gap', $oneXrun), $start1, @(120000, 'gap', (New-CauseXrun 2 0 120000)), @(120970, 'gap', $oneXrun)))
Check 'one cause=xrun per stream start across restarts passes' ($v.Pass -and $v.TotalXruns -eq 2 -and @($v.Excused).Count -eq 2)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-position-break' @($start0, @(30000, 'gap', 'cause=position_break callback=3000 frames=1440000 audio_ms=30000 ts_ns=1 dev_ns=2 pair_diff_ns=3')))
Check 'a cause=position_break gap event is reported in PositionBreaks and is not an xrun problem' ($v.Pass -and $v.TotalXruns -eq 0 -and @($v.PositionBreaks).Count -eq 1 -and $v.PositionBreaks[0].AudioMs -eq 30000)
$v = Get-WbXrunVerdict -SessionDir (New-XrunSession 'x-unknown-cause' @($start0, @(970, 'gap', 'cause=something_new n=1')))
Check 'an unknown cause=... gap event still fails as unreadable' (-not $v.Pass)

# ---- The gates run from any folder, and a native command's stderr cannot end them ------------
# A 3-hour load run and a 2-hour soak ended at their evidence step: the launcher was started from
# the user's home folder, `git rev-parse 2>$null` printed "fatal: not a git repository", and
# Windows PowerShell 5.1 turned that stderr line into a terminating NativeCommandError under
# $ErrorActionPreference = 'Stop'. The header now runs git with -C <repo root found from the
# module>, and every native call goes through Invoke-WbNative. This runs the header step in a
# Windows PowerShell 5.1 process whose current folder is outside the repository.
Import-Module (Join-Path $here '..\lib\PsEnv.psm1') -Force
$outside = Join-Path $env:TEMP ('wb-outside-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $outside | Out-Null
$probeGit = Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $outside, 'rev-parse', '--git-dir')
if ($probeGit.ExitCode -eq 0) {
  Write-Host 'skip outside-the-repo test: the temp folder is inside a git repository'
} else {
  $gateRunPath = (Resolve-Path (Join-Path $here '..\lib\GateRun.psm1')).Path
  $tpl = @'
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath '__OUTSIDE__'
Import-Module '__GATERUN__' -Force
$h = @(Get-WbGateHeader -Machine 'Legion')
$src = @($h | Where-Object { $_ -like 'source: commit *' })
[Console]::Out.WriteLine('HEADER_SOURCE_LINES=' + $src.Count)
[Console]::Out.WriteLine('HEADER_SOURCE_KNOWN=' + @($src | Where-Object { $_ -notlike '*unknown*' }).Count)
try { & git rev-parse --short HEAD 2>$null | Out-Null; [Console]::Out.WriteLine('CONTROL=no-throw') } catch { [Console]::Out.WriteLine('CONTROL=throws') }
$r = Invoke-WbNative -FilePath 'git' -ArgumentList @('rev-parse', '--short', 'HEAD')
[Console]::Out.WriteLine('NATIVE_FAILS=' + ($r.ExitCode -ne 0) + ' STDERR_CAPTURED=' + ($r.Error.Count -gt 0))
$m = Invoke-WbNative -FilePath 'no-such-command-wb.exe'
[Console]::Out.WriteLine('MISSING_EXIT=' + $m.ExitCode)
[Console]::Out.WriteLine('ROOT=' + (Get-WbRepoRoot))
'@
  $childPath = Join-Path $outside 'child.ps1'
  $tpl.Replace('__OUTSIDE__', $outside).Replace('__GATERUN__', $gateRunPath) | Set-Content -LiteralPath $childPath -Encoding ASCII
  $res = @(Invoke-WbWindowsPowerShell -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $childPath))
  $text = $res -join "`n"
  Write-Host ("info control (the old call, git ... 2>`$null under Stop, from outside the repo): " + (($res | Where-Object { $_ -like 'CONTROL=*' }) -join ''))
  Check 'the gate header step runs from a folder outside the repository (5.1, Stop)' ($LASTEXITCODE -eq 0 -and $text -match 'HEADER_SOURCE_LINES=1')
  Check 'the header names the commit even from outside the repository' ($text -match 'HEADER_SOURCE_KNOWN=1')
  Check 'a failing native command returns its exit code and stderr instead of throwing' ($text -match 'NATIVE_FAILS=True STDERR_CAPTURED=True')
  Check 'a missing native command returns -1 instead of throwing' ($text -match 'MISSING_EXIT=-1')
  Check 'the repository root comes from the module, not the current folder' ($text -match [regex]::Escape('ROOT=' + (Resolve-Path (Join-Path $here '..\..')).Path))
}
Remove-Item -LiteralPath $outside -Recurse -Force -ErrorAction SilentlyContinue

# No gate or library script may redirect a native command's stderr itself (that is the crash).
$scriptRoot = (Resolve-Path (Join-Path $here '..')).Path
$redirects = @(Get-ChildItem -Path (Join-Path $scriptRoot 'gates'), (Join-Path $scriptRoot 'lib') -Include *.ps1, *.psm1 -Recurse -File |
  Where-Object { $_.Name -notin @('GateRun.psm1', 'PsEnv.psm1') } |
  Where-Object { (Get-Content -LiteralPath $_.FullName) | Where-Object { $_ -match '\s2>' -and $_ -notmatch '^\s*#' } })
Check ('gate scripts redirect no native stderr themselves: ' + (($redirects | ForEach-Object { $_.Name }) -join ', ')) ($redirects.Count -eq 0)

# ---- Nothing timing-sensitive runs while an engine build is running ---------------------------------
$buildLock = Join-Path $tmp 'stelmar-build.lock'
$none = @('no-such-process-wb-test')
Check 'no build lock and no build process: nothing running' (@(Get-WbRunningBuilds -BuildLockPath $buildLock -ProcessNames $none).Count -eq 0)
Set-Content -LiteralPath $buildLock -Value 'build=test' -Encoding ascii
$threw = $false
try { Assert-WbNoBuilds -BuildLockPath $buildLock -ProcessNames $none } catch { $threw = $true; $msg = $_.Exception.Message }
Check 'the build lock stops a gate' ($threw -and $msg -match 'engine build is running' -and $msg -match 'exists')
Remove-Item -LiteralPath $buildLock -Force
# This very process stands in for a compiler: a process named like a build tool stops the gate too.
$me = (Get-Process -Id $PID).ProcessName
$threw = $false
try { Assert-WbNoBuilds -BuildLockPath $buildLock -ProcessNames @($me) } catch { $threw = $true; $msg = $_.Exception.Message }
Check 'a running build-tool process stops a gate' ($threw -and $msg -match 'pid')
# Invoke-WbGate refuses to start, and does not run the body or take the gate lock.
$ran = $false
$lock = Join-Path $tmp 'gate.lock'
$threw = $false
try { Invoke-WbGate -Name 'build-guard-test' -LockPath $lock -BuildLockPath $buildLock -BuildProcessNames @($me) -Body { $script:ran = $true } } catch { $threw = $true }
Check 'Invoke-WbGate refuses to start while a build runs, and takes no lock' ($threw -and -not $ran -and -not (Test-Path -LiteralPath $lock))
# Without a build it runs, and the header says so.
$ran = $false
Invoke-WbGate -Name 'build-guard-test' -LockPath $lock -BuildLockPath $buildLock -BuildProcessNames $none -MinFreeRamGB 0 -Body { $script:ran = $true }
Check 'Invoke-WbGate runs the body when no build is running' $ran
$hdr = (Get-WbGateHeader -Machine 'test' -Profile 'emu-low') -join "`n"
Check 'the evidence header says no engine build was running at the start and at the end' ($hdr -match 'engine builds: none running when the gate started' -and $hdr -match 'engine builds at the end: none running')

Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'GateRun tests passed'
