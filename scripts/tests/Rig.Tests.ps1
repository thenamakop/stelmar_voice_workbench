# Tests for scripts/lib/Rig.psm1: where the gates' player and reference capture run.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/Rig.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\GateRun.psm1') -Force
Import-Module (Join-Path $here '..\lib\Rig.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}
function Refuses([scriptblock]$Block, [string]$Pattern) {
  try { & $Block; return $false } catch { return ($_.Exception.Message -match $Pattern) }
}

# ---- Masks ---------------------------------------------------------------------------------
Check 'a hex mask parses' ((ConvertFrom-WbMask '0xf0000') -eq [uint64]0xf0000)
Check 'a mask formats back to hex' ((Format-WbMask ([uint64]0xc000)) -eq '0xc000')
Check 'logical processors are listed as ranges' ((Format-WbLogicalList ([uint64]0xf0000)) -eq '16-19')
Check 'logical processors list pairs and singles' ((Format-WbLogicalList ([uint64]0x5)) -eq '0,2')

# ---- The overlap check: overlapping masks are refused ---------------------------------------
$job = [uint64]0xf0000      # 16-19, the emu-low job
$player = [uint64]0xc000    # 14-15
$ref = [uint64]0x3000       # 12-13
$ok = $true
try { Assert-WbRigPlacement -JobMask $job -PlayerMask $player -ReferenceMask $ref } catch { $ok = $false }
Check 'disjoint masks are accepted' $ok
Check 'a player that shares a core with the job is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask ([uint64]0x30000) -ReferenceMask $ref } 'player.*shares logical processors 16-17 with the job')
Check 'a reference capture that shares a core with the job is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask $player -ReferenceMask ([uint64]0x80000) } 'reference capture.*shares logical processors 19 with the job')
Check 'a player and a reference capture on the same core are refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask $player -ReferenceMask ([uint64]0xc000) } 'share logical processors 14-15')
Check 'the refusal says the gate does not start' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask $job -ReferenceMask $ref } 'The gate does not start')
Check 'an empty player mask is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask ([uint64]0) -ReferenceMask $ref } 'player mask is empty')
Check 'a player pinned to every processor is refused' (Refuses { Assert-WbRigPlacement -JobMask $job -PlayerMask ([uint64]4294967295) -ReferenceMask $ref } 'shares logical processors')

# ---- The plan comes from wb-emulate (here a stand-in that prints its JSON) ---------------------
$tmp = Join-Path $env:TEMP ('wb-rig-tests-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
$fake = Join-Path $tmp 'fake-emulate.cmd'
Set-Content -LiteralPath $fake -Encoding ASCII -Value '@echo {"profile":"emu-low","job_mask":"0xf0000","player_mask":"0xc000","player_logical":[14,15],"reference_mask":"0x3000","reference_logical":[12,13],"strict":true,"note":null}'
$plan = Get-WbRigPlan -Emulate $fake -Profile 'emu-low'
Check 'the plan carries the job, player and reference masks' ($plan.JobMask -eq [uint64]0xf0000 -and $plan.PlayerMask -eq [uint64]0xc000 -and $plan.ReferenceMask -eq [uint64]0x3000 -and $plan.Strict)
Check 'the native profile has no plan' ($null -eq (Get-WbRigPlan -Emulate $fake -Profile 'native'))
$bad = Join-Path $tmp 'bad-emulate.cmd'
Set-Content -LiteralPath $bad -Encoding ASCII -Value "@echo no performance cores outside the mask 1>&2`r`n@exit /b 1"
Check 'a failing wb-emulate stops the gate with its message' (Refuses { Get-WbRigPlan -Emulate $bad -Profile 'emu-low' } 'wb-emulate rig --profile emu-low failed.*no performance cores')

# The real wb-emulate, when it is built: its plan must not overlap any emulation profile.
$realEmu = Join-Path $here '..\..\target\release\wb-emulate.exe'
if (Test-Path -LiteralPath $realEmu) {
  $realEmu = (Resolve-Path -LiteralPath $realEmu).Path
  $okAll = $true
  $union = [uint64]0
  foreach ($p in 'emu-low', 'emu-mid') { $union = $union -bor (Get-WbRigPlan -Emulate $realEmu -Profile $p).JobMask }
  foreach ($p in 'emu-low', 'emu-mid') {
    $rp = Get-WbRigPlan -Emulate $realEmu -Profile $p
    try { Assert-WbRigPlacement -JobMask $rp.JobMask -PlayerMask $rp.PlayerMask -ReferenceMask $rp.ReferenceMask } catch { $okAll = $false }
    if ((($rp.PlayerMask -bor $rp.ReferenceMask) -band $union) -ne 0 -and $rp.Strict) { $okAll = $false }
  }
  Check 'the real wb-emulate places the rig outside both emulation profiles' $okAll
} else {
  Write-Host 'skip real wb-emulate check: target\release\wb-emulate.exe is not built'
}

# ---- Pinning a real process, and reading back what it has ------------------------------------
$self = [System.Diagnostics.Process]::GetCurrentProcess()
$mine = [uint64]$self.ProcessorAffinity.ToInt64()
$lowest = $mine -band (-bnot ($mine - 1))
$child = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') -PassThru -WindowStyle Hidden
try {
  $placed = Set-WbRigPlacement -Process $child -Mask $lowest -Role 'player'
  Check 'a process is pinned and the mask is read back' ($placed.Mask -eq $lowest -and $placed.RequestedMask -eq $lowest)
  Check 'a process is set to HIGH priority class, not realtime' ($placed.Priority -eq 'High')
  Check "the process's CPU set is read back (none by default)" ($placed.CpuSet -match "^none")
} finally {
  Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue
}

# ---- Late callbacks are shown next to a shared break ------------------------------------------
$play = [pscustomobject]@{
  late_callbacks = 2; max_callback_interval_ms = 160.0
  callback_gaps_ge_20ms = 120; callback_gaps_ge_30ms = 9; callback_gaps_ge_50ms = 2
  late_callback_events = @(
    [pscustomobject]@{ source_ms = 52000.0; at_ms = 53000.0; interval_ms = 160.0; buffer_ms = 100.0 },
    [pscustomobject]@{ source_ms = 90000.0; at_ms = 91000.0; interval_ms = 120.0; buffer_ms = 100.0 })
}
$text = (Get-WbLateCallbackLines -Play $play -BreakSourceMs @(51978.7, 120000.0)) -join "`n"
Check 'the late-callback count, longest gap and gap histogram are shown' ($text -match 'player late callbacks: 2' -and $text -match 'longest gap between callbacks 160.0 ms' -and $text -match 'gaps of 20 ms or more: 120, 30 ms or more: 9, 50 ms or more: 2')
Check 'each late callback is listed with its source position' ($text -match 'late callback at source 52.000 s: 160.0 ms after the previous one \(output buffer 100 ms\)')
Check 'a shared break reports the late callbacks within 500 ms of it' ($text -match 'shared break at source 51.979 s: 1 late player callback\(s\) within 500 ms')
$quiet = [pscustomobject]@{ late_callbacks = 0; max_callback_interval_ms = 21.5; callback_gaps_ge_20ms = 120; callback_gaps_ge_30ms = 0; callback_gaps_ge_50ms = 0; late_callback_events = @() }
$qt = (Get-WbLateCallbackLines -Play $quiet -BreakSourceMs @(3877.0)) -join "`n"
Check 'a break with no late callback says the player never ran its buffer dry' ($qt -match 'no late player callback within 500 ms of it' -and $qt -match 'longest callback gap in the whole run was 21.5 ms' -and $qt -match 'never ran its output buffer dry')
$old = [pscustomobject]@{ underrun_callbacks = 0 }
Check 'an older play.json without the counter says so' (((Get-WbLateCallbackLines -Play $old) -join ' ') -match 'not recorded')

# ---- Reference xruns are shown next to a shared break -------------------------------------------
$refA = [pscustomobject]@{
  sample_rate = 48000; content_start_ms = 2278.0; content_first_source_frame = 0
  xruns = @(
    [pscustomobject]@{ n = 1; callbacks_before = 0; frames_before = 0; audio_ms = 0.0; wall_clock = '2026-11-09T20:34:08.912+05:30' },
    [pscustomobject]@{ n = 2; callbacks_before = 412; frames_before = 197760; audio_ms = 6150.0; wall_clock = '2026-11-09T20:34:13.790+05:30' })
}
$refR = [pscustomobject]@{ xruns = 2 }
$playAt = [DateTimeOffset]::Parse('2026-11-09T20:34:09.913+05:30')
$xt = (Get-WbReferenceXrunLines -RefAnalysis $refA -Reference $refR -PlayStarted $playAt -BreakSourceMs @(3877.3, 90000.0)) -join "`n"
Check 'each reference xrun is listed with its callback count, recording position and wall clock to the millisecond' ($xt -match '#2 at 20:34:13.790 IST: 412 callbacks and 197760 frames in; reference recording 6.150 s; about source 3.872 s')
Check 'an xrun before the signal starts says so' ($xt -match '#1 at 20:34:08.912 IST.*before the signal starts')
Check 'a shared break shows its wall-clock time to the second and the nearest reference xrun' ($xt -match 'shared break at source 3.877 s, 20:34:13.790 IST \(to the second: 20:34:13\): nearest reference xrun #2 at 20:34:13.790, within 1 ms of it')
Check 'a break with no reference xrun within 2 s says so' ($xt -match 'shared break at source 90.000 s.*no reference xrun within 2 s')
Check 'an earlier reference capture without the list says so' (((Get-WbReferenceXrunLines -RefAnalysis ([pscustomobject]@{ sample_rate = 48000; content_start_ms = 0.0; content_first_source_frame = 0 }) -Reference $refR -PlayStarted $playAt) -join ' ') -match 'no per-xrun list')

# ---- The go file: created after pinning, also without a plan ------------------------------------
$goFile = Join-Path $tmp 'GO-RIG'
$psExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$c1 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') -PassThru -WindowStyle Hidden
$c2 = Start-Process -FilePath $psExe -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') -PassThru -WindowStyle Hidden
try {
  $rigNone = Set-WbRigPins -Plan $null -Player $c1 -Reference $c2 -GoFile $goFile
  Check 'with no plan (native) nothing is pinned but the go file is created' ($null -eq $rigNone -and (Test-Path -LiteralPath $goFile))
  Remove-Item -LiteralPath $goFile -Force
  $own = [uint64]$self.ProcessorAffinity.ToInt64()
  $first = $own -band (-bnot ($own - 1))
  $rest = $own -bxor $first
  $second = $rest -band (-bnot ($rest - 1))
  $plan2 = [pscustomobject]@{ JobMask = [uint64]0; PlayerMask = $first; ReferenceMask = $(if ($second -ne 0) { $second } else { $first }); Strict = $true; Note = $null }
  $pinned = Set-WbRigPins -Plan $plan2 -Player $c1 -Reference $c2 -GoFile $goFile
  Check 'with a plan both processes are pinned at High and the go file appears after' ($pinned.Player.Priority -eq 'High' -and $pinned.Reference.Priority -eq 'High' -and (Test-Path -LiteralPath $goFile))
} finally {
  Stop-Process -Id $c1.Id, $c2.Id -Force -ErrorAction SilentlyContinue
}

# ---- The evidence lines show what the processes really had -------------------------------------
$rig = [pscustomobject]@{
  Plan   = [pscustomobject]@{ Note = $null }
  Job    = [pscustomobject]@{ Role = 'job (the app inside the emulation job)'; ProcessId = 10; Mask = [uint64]0xf0000; Priority = $null }
  Player = [pscustomobject]@{ Role = 'player'; ProcessId = 11; Mask = [uint64]0xc000; Priority = 'High' }
  Reference = [pscustomobject]@{ Role = 'reference capture'; ProcessId = 12; Mask = [uint64]0x3000; Priority = 'High' }
}
$rl = (Get-WbRigLines -Rig $rig -Play ([pscustomobject]@{ affinity_mask = '0xc000' }) -Reference ([pscustomobject]@{ affinity_mask = '0x3000' })) -join "`n"
Check 'the evidence lists the job, player and reference masks with logical processors' ($rl -match 'mask 0xf0000 \(logical processors 16-19\)' -and $rl -match 'player: pid 11, mask 0xc000 \(logical processors 14-15\), priority class High' -and $rl -match 'reference capture: pid 12, mask 0x3000')
Check 'the evidence shows what the player and the reference report themselves' ($rl -match 'player itself reports 0xc000' -and $rl -match 'reference capture itself reports 0x3000')
$rig.Player | Add-Member -NotePropertyName CpuSet -NotePropertyValue "none (all processors the affinity mask allows)"
$rig.Reference | Add-Member -NotePropertyName CpuSet -NotePropertyValue "12-13"
$rl2 = (Get-WbRigLines -Rig $rig -Play $null -Reference $null) -join "`n"
Check "the evidence shows each process's CPU set next to its mask" ($rl2 -match 'player: pid 11, mask 0xc000 \(logical processors 14-15\), priority class High; CPU set none' -and $rl2 -match 'reference capture: pid 12, mask 0x3000 \(logical processors 12-13\), priority class High; CPU set 12-13')

Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
# ---- The label says whose break it is (12 Nov load FAIL: app-only breaks were labelled "shared") ----
$lblPlay = [pscustomobject]@{ late_callbacks = 0; max_callback_interval_ms = 21.4; callback_gaps_ge_20ms = 5; callback_gaps_ge_30ms = 0; callback_gaps_ge_50ms = 0; late_callback_events = @() }
$lt = (Get-WbLateCallbackLines -Play $lblPlay -BreakSourceMs @(47049.0) -Label 'app-only break (not in the reference capture)') -join "`n"
Check 'a break given another label is not called a shared break (late callbacks)' ($lt -match 'app-only break \(not in the reference capture\) at source 47.049 s' -and $lt -notmatch 'shared break')
$lx = (Get-WbReferenceXrunLines -RefAnalysis $refA -Reference $refR -PlayStarted $playAt -BreakSourceMs @(90000.0) -Label 'app-only break (not in the reference capture)') -join "`n"
Check 'a break given another label is not called a shared break (reference xruns)' ($lx -match 'app-only break \(not in the reference capture\) at source 90.000 s' -and $lx -notmatch 'shared break at')
Check 'the default label stays shared break' ($text -match 'shared break at source 51.979 s')
# The load gate reads the stress processes' priority class and threads from the running processes.
Check 'a stress command line is told apart: in the job (--gpu), on the host (--threads), other commands are not stress' (((Get-WbStressKind 'wb-testbench.exe stress --gpu Intel --seconds 9') -eq 'job') -and ((Get-WbStressKind 'wb-testbench.exe stress --threads 32 --seconds 9') -eq 'host') -and ($null -eq (Get-WbStressKind 'wb-testbench.exe play --device x')) -and ($null -eq (Get-WbStressKind '')))
$okJob = [pscustomobject]@{ Kind = 'job'; Id = 11; PriorityClass = 'Normal'; Threads = 6; BasePriorities = @('6 thread(s) at base priority 8'); AllThreadsNormal = $true; CoresBusy = 3.9; Affinity = '0xF0000'; SampleSeconds = 3 }
$okHost = [pscustomobject]@{ Kind = 'host'; Id = 12; PriorityClass = 'Normal'; Threads = 33; BasePriorities = @('33 thread(s) at base priority 8'); AllThreadsNormal = $true; CoresBusy = 28.5; Affinity = '0xFFFFFFFF'; SampleSeconds = 3 }
$sl = (Get-WbStressLines -Info @($okJob, $okHost)) -join "`n"
Check 'normal stress is reported with its count, class, thread priorities and cores busy' ($sl -match 'inside the job: pid 11, priority class Normal, 6 threads' -and $sl -match 'on the host, outside the job: pid 12, priority class Normal, 33 threads' -and $sl -match '3\.9 cores busy over 3 s' -and $sl -match 'every stress process is at normal priority')
$high = [pscustomobject]@{ Kind = 'job'; Id = 13; PriorityClass = 'High'; Threads = 6; BasePriorities = @('6 thread(s) at base priority 13'); AllThreadsNormal = $false; CoresBusy = 4.0; Affinity = '0xF0000'; SampleSeconds = 3 }
$hl = (Get-WbStressLines -Info @($high, $okHost)) -join "`n"
Check 'a stress process that is not at normal priority is called out' ($hl -match 'NOT NORMAL PRIORITY' -and $hl -match 'the load gate requires normal priority')
Check 'no stress process found is said plainly' (((Get-WbStressLines -Info @()) -join '') -match 'no stress process was found')
$tb = @('target\debug\wb-testbench.exe', 'target\release\wb-testbench.exe') | ForEach-Object { Join-Path (Join-Path $here '..\..') $_ } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if ($tb) {
  $stop = Join-Path ([IO.Path]::GetTempPath()) ('wb-stress-stop-' + [guid]::NewGuid().ToString('N'))
  $sp = Start-Process -FilePath $tb -ArgumentList @('stress', '--threads', '2', '--seconds', '20', '--stop-file', $stop) -PassThru -WindowStyle Hidden
  try {
    Start-Sleep -Seconds 2
    $live = @(Get-WbStressInfo -SampleSeconds 2 | Where-Object { $_.Id -eq $sp.Id })
    Check 'a real stress process is found and read: host kind, normal class, about 2 cores busy' ($live.Count -eq 1 -and $live[0].Kind -eq 'host' -and $live[0].PriorityClass -eq 'Normal' -and $live[0].AllThreadsNormal -and $live[0].CoresBusy -gt 1.5 -and $live[0].CoresBusy -lt 2.5)
  } finally {
    Set-Content -LiteralPath $stop -Value 'stop'
    if (-not $sp.WaitForExit(15000)) { Stop-Process -Id $sp.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $stop -Force -ErrorAction SilentlyContinue
  }
} else {
  Write-Host 'skip live stress read: wb-testbench.exe is not built'
}

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'Rig tests passed'
