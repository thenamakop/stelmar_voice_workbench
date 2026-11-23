# Gate queue-soak: the recorder beside the app's real transcription queue. Two sessions through the rig: public
# speech that the queue transcribes while the second session (seeded noise) is recorded. Wraps
# scripts\gates\queue-soak.ps1; the verdict rules are in scripts\lib\QueueSoak.psm1.
#
# It needs the capability.json that THIS pack run's quick check kept: without a system check the app creates no
# job. No such file (the quick check has not run on this laptop, or kept none) is PENDING with that reason, never
# FAIL; the runner orders quick-check before this gate (depends_on in gates.manifest.json). The app gets a data
# folder of its own, the capability.json is copied into it, and nothing of the laptop's real settings is touched.
# Windows PowerShell 5.1, ASCII only.
param([string]$CtxFile, [string]$ResultFile)
$ErrorActionPreference = 'Stop'
$lib = Join-Path $PSScriptRoot '..\..\lib'
Import-Module (Join-Path $lib 'PackCore.psm1') -Force
Import-Module (Join-Path $lib 'PackGate.psm1') -Force
$ctx = Read-PackCtx -CtxFile $CtxFile
Initialize-PackGate -Ctx $ctx
Import-Module (Join-Path $lib 'RecorderGates.psm1') -Force
Import-Module (Join-Path $lib 'LoopbackRig.psm1') -Force
Import-Module (Join-Path $lib 'QueueSoak.psm1') -Force
Invoke-PackGateBody -Ctx $ctx -Id 'queue-soak' -ResultFile $ResultFile -Body {
  $quick = [bool]$ctx.quick
  $paths = Get-PackPaths -Ctx $ctx
  $threshold = 'recording rule for both sessions (one offset per run, 0 dropped, at most 9 counted breaks per 90 min, equals both references); 0 overruns, 0 xruns; session 1 transcribed in full (job done, no gap, no overlap, no duplicate); decoding of session 1 overlapped session 2'
  # The quick check of this pack run keeps its capability.json in its own gate folder (quick-check.ps1).
  $capPath = Join-Path (Join-Path (Join-Path ([string]$ctx.work_dir) 'gates') 'quick-check') 'capability.json'
  $cap = Get-WbQueueSoakCapability -Path $capPath
  if (-not $cap.Found) {
    return (New-PackResult -Id 'queue-soak' -Status 'PENDING' -Headline $cap.Reason -Threshold $threshold -Quick $quick)
  }
  foreach ($f in @($paths.GateExe, $paths.Testbench, $paths.BenchClips)) { if (-not (Test-Path -LiteralPath $f)) { throw "the pack is missing $f (run run-gates.ps1 -Verify)" } }
  $settings = Get-WbQueueSoakSettings -Quick:$quick
  $minutes = [math]::Ceiling(($settings.Session1Seconds + $settings.Session2Seconds) / 60.0 + $settings.DecodeWaitMinutes + 15)
  Assert-PackFreeDisk -Path ([string]$ctx.work_dir) -NeedGB (Get-PackDiskNeedGB -Minutes $minutes) -What 'the queue-soak recordings'
  Assert-PackBackupDrive -BackupRoot ([string]$ctx.backup_root)
  $args1 = @(Get-PackLegacyCommon -Ctx $ctx) + @('-RigPrefer', [string]$ctx.rig_prefer, '-CapabilityFile', (ConvertTo-PackArg $capPath), '-BenchClips', (ConvertTo-PackArg $paths.BenchClips))
  if ($quick) { $args1 += '-Quick' }
  $started = Get-Date
  $e = Invoke-PackLegacyGate -Ctx $ctx -Script 'queue-soak.ps1' -Arguments $args1 -EvidenceName ("queue-soak-$([string]$ctx.profile).txt") -TimeoutMinutes ([int]($settings.MaxMinutes + 30))
  $wall = ((Get-Date) - $started).TotalMinutes
  if ($e.TimedOut) { return (New-PackResult -Id 'queue-soak' -Status 'INVALID' -Headline 'the queue-soak gate did not finish in time' -Evidence @($e.Log) -Threshold $threshold -Quick $quick) }
  $evLines = @()
  if ($e.Evidence) { $evLines = @(Get-Content -LiteralPath $e.Evidence -Encoding UTF8) }
  # A refusal before the run (rig, overlay tools, RAM, a missing system check) is in the crash evidence; pass its reason on.
  $reason = @($evLines | Where-Object { $_ -cmatch 'the queue-soak gate stopped on an error: (RIG REFUSED|RIG SELF-TEST FAILED|Overlay tools are running|Only [0-9.]+ GB|An engine build|NEEDS A SYSTEM CHECK)' } | Select-Object -First 1)
  if ($reason.Count -gt 0 -and $e.ExitCode -ne 0) {
    $msg = [string]$reason[0]
    if ($msg -match 'stopped on an error: (.+)$') { $msg = $Matches[1] }
    $st = $(if ($msg -match '^NEEDS A SYSTEM CHECK') { 'PENDING' } else { Get-PackErrorStatus $msg })
    return (New-PackResult -Id 'queue-soak' -Status $st -Headline $msg -Evidence @($e.Evidence) -Threshold $threshold -Quick $quick)
  }
  $rigKind = Get-PackRigKindFromEvidence -EvidenceLines $evLines
  $rigLabel = Get-PackRigLabel -Rig ([pscustomobject]@{ Kind = $(if ($rigKind -eq 'loopback') { 'loopback' } else { 'vb-cable' }); Fallback = ($rigKind -eq 'vb-cable-fallback'); Instance = '' })
  if ($e.ExitCode -eq 4) {
    $head = 'not applicable on this laptop: its system check did not clear processing while recording, so the queue correctly waits for the recording to end (nothing to measure)'
    return (New-PackResult -Id 'queue-soak' -Status 'INFO' -Headline $head -Evidence @($e.Evidence) -Threshold $threshold -Quick $quick -Rig $rigLabel)
  }
  $out = Get-PackRecorderOutcome -ExitCode $e.ExitCode -EvidenceLines $evLines
  $head = $(if ($out.VerdictLine) { $out.VerdictLine -replace '^verdict:\s*', '' } else { $out.Status })
  if ($out.Status -eq 'FAIL') {
    $failed = @(Get-PackFailedChecks -EvidenceLines $evLines)
    if ($failed.Count -gt 0 -and $head -notmatch [regex]::Escape($failed[0])) { $head = ($head + ' [failed checks: ' + ($failed -join '; ') + ']') }
  }
  if ($rigKind -eq 'vb-cable-fallback') { $head = "$head [VB-CABLE fallback: does not close the hardware-interface rows]" }
  # The numbers the owner asked for, taken from the evidence the gate wrote.
  $num = { param($pattern) $m = @($evLines | ForEach-Object { [regex]::Match([string]$_, $pattern) } | Where-Object { $_.Success } | Select-Object -First 1); if ($m.Count) { $m[0].Groups[1].Value } else { $null } }
  $measured = [ordered]@{
    mode = $settings.Mode; rig = $rigKind; exit_code = $e.ExitCode; wall_minutes = [math]::Round($wall, 1)
    gate_minutes = (& $num 'real duration of the gate: ([0-9.,]+) min')
    decoding_inside_session_2_s = (& $num 'decoding of session 1 inside the recording of session 2: ([0-9,.]+) s')
    ready_by_error_min = (& $num 'error ([+-]?\d+) min \(clock minutes\)')
    engine_peak_mib = (& $num 'engine memory peak while session 1 was decoded: (\d+) MiB')
    run_dir = (Get-PackRunDirFromEvidence -EvidenceLines $evLines)
  }
  New-PackResult -Id 'queue-soak' -Status $out.Status -Headline $head -Measured $measured -Threshold $threshold -Rig $rigLabel -Evidence @($e.Evidence) -Quick $quick
}
