# Tests for scripts/lib/Sampler.psm1 and scripts/gates/xrun-correlate.ps1: a short real sampler run
# and a made-up session.log with xruns inside it. No audio is played.
# Run: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/XrunCorrelate.Tests.ps1
# ASCII only; Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PsEnv.psm1') -Force
Import-Module (Join-Path $here '..\lib\GateRun.psm1') -Force
Import-Module (Join-Path $here '..\lib\Sampler.psm1') -Force
$fails = 0
function Check([string]$Name, [bool]$Ok) {
  if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name"; $script:fails++ }
}

$counters = @(Get-WbSamplerCounters)
Check 'the sampler reads the job cores one by one' (($counters -contains '\Processor Information(0,16)\% DPC Time') -and ($counters -contains '\Processor Information(0,19)\% Processor Performance'))
Check 'the sampler reads per-process CPU with the process id, disks, memory and network' (($counters -contains '\Process(*)\% Processor Time') -and ($counters -contains '\Process(*)\ID Process') -and ($counters -contains '\PhysicalDisk(*)\Avg. Disk sec/Write') -and ($counters -contains '\Memory\Available MBytes') -and ($counters -contains '\Network Interface(*)\Bytes Total/sec'))
Check 'other job cores can be asked for' (@(Get-WbSamplerCounters -JobProcessors @(8)) -contains '\Processor Information(0,8)\% Interrupt Time')

$work = Join-Path ([IO.Path]::GetTempPath()) ('wb-xc-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$p = $null
try {
  $csv = Join-Path $work 'sampler.csv'
  $started = Get-Date
  $p = Start-WbSampler -OutCsv $csv -Seconds 7
  Check 'the sampler is pinned outside the job cores (logical processors 0-11) and runs at High priority' (($p.ProcessorAffinity.ToInt64() -eq 0xFFF) -and ($p.PriorityClass -eq 'High'))
  [void]$p.WaitForExit(40000)
  $r = Read-WbSamplerCsv -Path $csv
  Check 'the CSV is read: about 7 rows and hundreds of counters' ($r.Rows.Count -ge 5 -and $r.Names.Count -gt 100)
  Check 'row times are the machine clock' ($r.Rows[0].Time -ge $started.AddSeconds(-5) -and $r.Rows[-1].Time -le (Get-Date).AddSeconds(5))
  Check 'a job core column exists and has a number' ((@($r.Names | Where-Object { $_ -eq '\Processor Information(0,16)\% Processor Time' }).Count -eq 1))

  # A session.log with two xruns inside the sampled seconds, each followed by its position break.
  $sess = Join-Path $work 'sess'
  New-Item -ItemType Directory -Path $sess | Out-Null
  $t1 = $r.Rows[2].Time.AddMilliseconds(500); $t2 = $r.Rows[4].Time.AddMilliseconds(200)
  function Stamp([datetime]$t) { ([DateTimeOffset]$t).ToString('yyyy-MM-ddTHH:mm:ss.fffffffzzz') }
  $logLines = @(
    ((Stamp $t1) + ' gap cause=xrun n=1 callback=100 frames=48000 audio_ms=1000 qpc=1 qpc_hz=10000000'),
    ((Stamp $t1.AddMilliseconds(20)) + ' position_break callback=102 frames=48960 audio_ms=1020 ts_ns=1 dev_ns=310000000 pair_diff_ns=0'),
    ((Stamp $t2) + ' gap cause=xrun n=2 callback=200 frames=96000 audio_ms=2000 qpc=1 qpc_hz=10000000'),
    ((Stamp $t2.AddMilliseconds(20)) + ' position_break callback=202 frames=96960 audio_ms=2020 ts_ns=1 dev_ns=130000000 pair_diff_ns=0'))
  Set-Content -LiteralPath (Join-Path $sess 'session.log') -Value $logLines -Encoding ascii
  $out = Join-Path $work 'out\correlation.txt'
  $xc = Join-Path $here '..\gates\xrun-correlate.ps1'
  $res = @(Invoke-WbWindowsPowerShell -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $xc, '-RunDir', $work, '-SessionDir', $sess, '-SamplerCsv', $csv, '-Out', $out, '-Machine', 'TestMachine'))
  Check 'the correlation script runs and writes its file' ((Test-Path -LiteralPath $out) -and ($LASTEXITCODE -eq 0))
  if (Test-Path -LiteralPath $out) {
    $txt = Get-Content -LiteralPath $out -Raw
    Check 'it finds both xruns' ($txt -match 'driver xruns in the app''s session.log: 2')
    Check 'it reads how long the capture thread was away from the position break (310 ms and 130 ms)' ($txt -match 'xrun #1 at .*about 310 ms' -and $txt -match 'xrun #2 at .*about 130 ms')
    Check 'it lists the job cores one by one for each xrun' ((@([regex]::Matches($txt, 'cpu16: busy')).Count -ge 3) -and $txt -match 'cpu19: busy')
    Check 'it lists the busiest processes' ($txt -match 'busiest processes in the window')
    Check 'it compares with the other seconds and reads the System log' ($txt -match 'other seconds of the run' -and $txt -match 'System log during the sampler' -and $txt -match 'event 37')
    Check 'it says what a one-second sampler cannot show' ($txt -match 'averaged')
  }
  $lines = @(Get-WbMachineStateLines)
  Check 'the machine state has uptime, power plan and mains lines' (($lines -join "`n") -match 'machine state at start: up \d' -and ($lines -join "`n") -match 'power plan:' -and ($lines -join "`n") -match 'mains power:')
} finally {
  if ($p) { Stop-WbSampler -Process $p }
  Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

# The gate script wires it in, and a diagnostic is labelled as one.
$g = Get-Content -LiteralPath (Join-Path $here '..\gates\recorder-gate.ps1') -Raw
Check 'recorder-gate.ps1 has -Sample and -DiagnosticNote' ($g -match '\[switch\]\$Sample' -and $g -match '\[string\]\$DiagnosticNote')
Check 'a diagnostic run says DIAGNOSTIC, NOT A VERDICT in the evidence' ($g -match 'verdict: DIAGNOSTIC, NOT A VERDICT')
Check 'a diagnostic run never exits with the gate''s FAIL code' ($g -match 'if \(\$DiagnosticNote\) \{ exit 0 \}')
Check 'the sampler is stopped by the cleanup after an error too' ($g -match 'Stop-WbRigLeftovers -Process @\(\$emu, \$play, \$refProc, \$hostStress, \$sampler\)')
Check 'machine state (uptime, power plan, mains) is written for every run' ($g -match 'Get-WbMachineStateLines' -and $g -match 'MachineState')

if ($fails) { Write-Host "$fails failed"; exit 1 }
Write-Host 'XrunCorrelate tests passed'
