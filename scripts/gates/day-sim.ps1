<#
.SYNOPSIS
  Phase 4 day simulation. So far only -Smoke exists: a short run (about 35 minutes) that is NOT a gate; it makes the
  paths of the post-processing queue run together once with the real engines before the full day is started.

.DESCRIPTION
  -Smoke runs the gate build of the app (target\gate, cargo feature gate-hooks; never an installer build) inside the
  emulation job (wb-emulate, default emu-low) with WB_GATE_DAY_PLAN. The app records three short sessions from the
  virtual cable (scripts\gates\make_day_audio.py --smoke builds the audio: English, Hindi, and a session that changes
  from English to Hindi and back), one wb-testbench player per session, started outside the job when the app logs the
  session's start. The plan also does, at fixed moments:

    - queue_pause and queue_resume after the first session stopped (the operator's pause and resume),
    - set_language_lock to Hindi and back to detection inside the third session (the lock must reach the lang_log
      of the unit that covers it: read from the batch request files afterwards),
    - import_session of a copy of the first finished session (a folder as it would arrive from another computer).

  The script itself kills one batch engine process from outside (taskkill, as the supervisor kill test does) once an
  engine of the second or third session has run for 20 s. The night pass is made to start 60 s after the last
  session (WB_GATE_NIGHT_WAIT_S, gate builds only) and is stopped after -NightMinutes minutes: "a short night pass".

  The saved profile of a full system check (D:\STELMAR-Gate\system-check\full-emu-low\data) is copied into a data
  folder of this run, so the real profile is never touched. The app's own view of it must show no fingerprint
  change, or the run stops (a stale profile would decide with safe defaults and test nothing else).

  The run holds D:\Projects\stelmar-gate.lock (Invoke-WbGate), refuses to start while overlay tools or builds run,
  and the evidence goes through scripts\lib\Redact.psm1. Audio stays under D:\STELMAR-Gate and C:\STELMAR-Gate-Backup,
  never in the repository.

  Needs: VB-CABLE (run scripts\gates\rig-selftest.ps1 first after a restart), the gate build
  (scripts\gates\build-gate.ps1), the engines in src-tauri\binaries, wb-testbench and wb-emulate release builds, the
  models under models\, and the smoke audio (eval\.venv\Scripts\python scripts\gates\make_day_audio.py --smoke).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\day-sim.ps1 -Smoke
  powershell -ExecutionPolicy Bypass -File .\scripts\gates\day-sim.ps1 -Smoke -FromRun D:\STELMAR-Gate\day-sim\smoke-emu-low-20261114-231500
#>
[CmdletBinding()]
param(
  [switch]$Smoke,
  [ValidateSet('emu-low', 'emu-mid')] [string]$Profile = 'emu-low',
  [string]$Machine = 'Legion',
  [string]$WorkRoot = 'D:\STELMAR-Gate\day-sim',
  [string]$BackupRoot = 'C:\STELMAR-Gate-Backup',
  [string]$ProfileData = 'D:\STELMAR-Gate\system-check\full-emu-low\data',
  [string]$Audio = 'D:\STELMAR-Gate\day-sim\smoke',
  [int]$NightMinutes = 8,
  [int]$PartSeconds = 120,
  [string]$CaptureDevice = '',
  [string]$PlayDevice = '',
  [string]$OutDir,
  [string]$Exe,
  [string]$Testbench,
  [string]$Emulate,
  [switch]$AllowCableDefault,
  # Write the evidence again from a finished run's folder; start nothing.
  [string]$FromRun
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
if (-not $Smoke) { throw 'only -Smoke exists so far: the full day simulation is built after the owner says go' }
if (-not $OutDir) { $OutDir = Join-Path $repo 'docs\evidence\phase-4' }
if (-not $Exe) { $Exe = Join-Path $repo 'target\gate\stelmar-voice-workbench.exe' }
if (-not $Testbench) { $Testbench = Join-Path $repo 'target\release\wb-testbench.exe' }
if (-not $Emulate) { $Emulate = Join-Path $repo 'target\release\wb-emulate.exe' }
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Redact.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\AppProcess.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\Rig.psm1') -Force
Import-Module (Join-Path $repo 'scripts\lib\CableRig.psm1') -Force
$python = Join-Path $repo 'eval\.venv\Scripts\python.exe'
New-Item -ItemType Directory -Force -Path $OutDir, $WorkRoot | Out-Null

function Q([string]$v) { ConvertTo-WbArg $v }

function Read-Json([string]$Path) {
  if (Test-Path -LiteralPath $Path) { Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
}

function Write-Utf8([string]$Path, [string]$Text) {
  [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding($false)))
}

# The new lines of a text file another process appends to (shared read).
function Read-NewLines([string]$Path, [ref]$Count) {
  if (-not (Test-Path -LiteralPath $Path)) { return @() }
  $fs = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
  try {
    $sr = New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8)
    $all = @(($sr.ReadToEnd()) -split "`r?`n" | Where-Object { $_ -ne '' })
  } finally { $fs.Dispose() }
  $new = @($all | Select-Object -Skip $Count.Value)
  $Count.Value = $all.Count
  $new
}

function Write-SmokeEvidence([string]$Run) {
  $tmp = Join-Path $Run 'evidence.txt'
  $rep = Invoke-WbNative -FilePath $python -ArgumentList @('-X', 'utf8', (Join-Path $repo 'scripts\gates\day_sim_report.py'), $Run, $tmp)
  if (-not (Test-Path -LiteralPath $tmp)) { throw "the report was not written: $((@($rep.Error) + @($rep.Output)) -join ' ')" }
  $body = @([IO.File]::ReadAllLines($tmp, [Text.Encoding]::UTF8))
  $out = Join-Path $OutDir "day-sim-smoke-$Profile.txt"
  $header = @(
    "date: $(Get-Date -Format 'yyyy-MM-dd HH:mm') (IST on the Legion)",
    "command: powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\day-sim.ps1 -Smoke -Profile $Profile",
    "machine: $Machine, emulation profile $Profile; the gate build target\gate\stelmar-voice-workbench.exe, WB_GATE_DAY_PLAN; NOT A GATE (a smoke run of the queue paths)",
    "source: $((Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $repo, 'rev-parse', '--short', 'HEAD')).Output -join '')",
    "build: $(Split-Path -Leaf $Exe) sha256 $((Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash.ToLowerInvariant())",
    "run folder: $Run"
  )
  Write-WbEvidence -Path $out -Lines ($header + $body)
  Write-Host "evidence: $out"
}

if ($FromRun) {
  Write-SmokeEvidence (Resolve-Path $FromRun).Path
  return
}

foreach ($f in @($Exe, $Testbench, $Emulate)) { if (-not (Test-Path -LiteralPath $f)) { throw "Missing $f; build it first (scripts\gates\build-gate.ps1)" } }
$audioManifest = Join-Path $Audio 'smoke-audio.json'
if (-not (Test-Path -LiteralPath $audioManifest)) { throw "no smoke audio at $Audio; run: eval\.venv\Scripts\python scripts\gates\make_day_audio.py --smoke" }
foreach ($f in @('capability.json')) { if (-not (Test-Path -LiteralPath (Join-Path $ProfileData $f))) { throw "no saved profile at $ProfileData (run scripts\gates\system-check-gate.ps1 -Runs full-emu-low)" } }
$sessions = @((Get-Content -LiteralPath $audioManifest -Raw -Encoding UTF8 | ConvertFrom-Json).sessions)

$failed = $null
try {
  $result = Invoke-WbGate -Name "day-sim smoke $Profile" -Body {
    if (Get-Process stelmar-voice-workbench -ErrorAction SilentlyContinue) { throw 'The app is already running; close it first.' }
    # The rig first (see recorder-gate.ps1): paired endpoints, the cable not the default device, nobody else playing
    # into it, a known signal heard on the capture.
    Assert-WbNoResidentAudio
    $cableRig = Resolve-WbCableRig -Testbench $Testbench -PlayDevice $PlayDevice -CaptureDevice $CaptureDevice
    $null = @(Assert-WbCableNotDefault -Testbench $Testbench -Rig $cableRig -Allow:$AllowCableDefault)
    $intruders = @(Get-WbCableIntruders -Testbench $Testbench -Names $cableRig.RenderNames)
    if ($intruders.Count -gt 0) { throw ("RIG REFUSED: another program is playing into the cable: " + ($intruders -join ', ')) }
    $workRootForTest = Split-Path -Parent $WorkRoot
    $cableTest = Invoke-WbRigSelfTest -Rig $cableRig -Testbench $Testbench -WorkRoot $workRootForTest
    $playName = $cableTest.PlayName
    $capName = $cableTest.CaptureName
    $rigPlan = Get-WbRigPlan -Emulate $Emulate -Profile $Profile
    if ($rigPlan) { Assert-WbRigPlacement -JobMask $rigPlan.JobMask -PlayerMask $rigPlan.PlayerMask -ReferenceMask $rigPlan.ReferenceMask -What 'planned, before anything starts' }

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $run = Join-Path $WorkRoot "smoke-$Profile-$stamp"
    $rec = Join-Path $run 'rec'
    $data = Join-Path $run 'data'
    $backup = Join-Path $BackupRoot "smoke-$Profile-$stamp"
    New-Item -ItemType Directory -Force -Path $rec, $data | Out-Null
    Copy-Item -LiteralPath (Join-Path $ProfileData 'capability.json') -Destination $data
    if (Test-Path -LiteralPath (Join-Path $ProfileData 'verified-models.json')) { Copy-Item -LiteralPath (Join-Path $ProfileData 'verified-models.json') -Destination $data }
    Write-Host "$(Get-Date -Format 'HH:mm:ss') run folder $run"

    function Start-Emulated([string]$Mode, [int]$LimitMinutes) {
      $emu = Start-Process -FilePath $Emulate -ArgumentList @('--profile', $Profile, '--log', (Q (Join-Path $run "emulate-$Mode.log")), '--', (Q $Exe)) -PassThru -WindowStyle Hidden
      $emu
    }

    # 1. The app's own view of the saved profile: a fingerprint change would decide with safe defaults.
    $env:WB_GATE_DATA_DIR = $data
    $env:WB_GATE_ROOT = $rec
    $env:WB_GATE_CHECK = 'view'
    $env:WB_GATE_REPORT = Join-Path $run 'view.json'
    $view = Start-Emulated 'view' 5
    if (-not $view.WaitForExit(300000)) { Stop-Process -Id $view.Id -Force; throw 'the profile view did not finish within 5 minutes' }
    Remove-Item Env:\WB_GATE_CHECK -ErrorAction SilentlyContinue
    $v = Read-Json (Join-Path $run 'view.json')
    if (-not $v -or -not $v.view.capability) { throw "the app could not read the saved profile (see $run\view.json)" }
    $changes = @($v.view.fingerprint_changes)
    $d = $v.view.capability.decision
    Write-Host ("{0} profile: tier {1}, config {2}, cpu threads {3}, process while recording {4}, fingerprint changes: {5}" -f (Get-Date -Format 'HH:mm:ss'), $d.tier, ($d.config.config | ConvertTo-Json -Compress), $d.config.cpu_threads, $d.process_while_recording, $changes.Count)
    if ($changes.Count -gt 0) { throw "the saved profile no longer matches this build (fingerprint changes: $($changes -join '; ')); run the full check again (scripts\gates\system-check-gate.ps1 -Runs full-emu-low)" }

    # 2. The plan.
    $at = 0
    $planSessions = @()
    foreach ($s in $sessions) {
      $dur = [int][math]::Ceiling([double]$s.seconds) + 25
      $planSessions += [ordered]@{ title = $s.title; at_s = $at; duration_s = $dur; languages = @($s.languages); device = $capName }
      $at += $dur + 30
    }
    $first = $sessions[0].title
    $mixed = $sessions[2].title
    $actions = @(
      [ordered]@{ session = $first; on = 'stop'; after_s = 6; do = 'queue_pause' },
      [ordered]@{ session = $first; on = 'stop'; after_s = 50; do = 'queue_resume' },
      [ordered]@{ session = $mixed; on = 'start'; after_s = 170; do = 'lock_language'; lang = 'hi' },
      [ordered]@{ session = $mixed; on = 'start'; after_s = 230; do = 'lock_language' },
      [ordered]@{ session = $first; on = 'stop'; after_s = 120; do = 'import_session'; from = $first }
    )
    $planFile = Join-Path $run 'day-plan.json'
    Write-Utf8 $planFile ((@{ sessions = $planSessions; actions = $actions } | ConvertTo-Json -Depth 6))

    # 3. The run: the first session of the plan is 10:00 IST on the scheduler's clock.
    $nowIst = Get-Date
    $ten = (Get-Date -Hour 10 -Minute 0 -Second 0 -Millisecond 0)
    $offset = [int](($ten - $nowIst).TotalSeconds)
    $stopFile = Join-Path $run 'STOP'
    $queueLog = Join-Path $run 'queue.log'
    $env:WB_GATE_DAY_PLAN = $planFile
    $env:WB_GATE_DAY_STOP_FILE = $stopFile
    $env:WB_GATE_REPORT = Join-Path $run 'report.json'
    $env:WB_GATE_BACKUP = $backup
    $env:WB_GATE_QUEUE_LOG = $queueLog
    $env:WB_GATE_CLOCK_OFFSET_S = "$offset"
    $env:WB_GATE_NIGHT_WAIT_S = '60'
    $env:WB_GATE_PART_SECONDS = "$PartSeconds"
    $emu = $null
    $players = @()
    $kill = $null
    $started = Get-Date
    $hardLimit = $started.AddMinutes(100)
    $lines = 0
    $startedTitles = @{}
    $allRecorded = $null
    $nightPlanned = $null
    $nightSkipped = $null
    $lastProgress = Get-Date
    $lastLine = ''
    $progress = New-Object System.Collections.Generic.List[string]
    try {
      $emu = Start-Emulated 'day' 100
      while (-not $emu.HasExited) {
        $ref = [ref]$lines
        foreach ($l in (Read-NewLines $queueLog $ref)) {
          $lastLine = $l
          if ($l -match 'DAY: started "(.+?)" in') {
            $title = $Matches[1]
            if (-not $startedTitles.ContainsKey($title)) {
              $startedTitles[$title] = Get-Date
              $s = @($sessions | Where-Object { $_.title -eq $title })[0]
              $i = $startedTitles.Count
              $goFile = Join-Path $run "GO-$i"
              $play = Start-Process -FilePath $Testbench -ArgumentList @('play', '--device', (Q $playName), '--file', (Q (Join-Path $Audio $s.file)), '--go-file', (Q $goFile), '--report', (Q (Join-Path $run "play-$i.json"))) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $run "play-$i.out") -RedirectStandardError (Join-Path $run "play-$i.err")
              if ($rigPlan) { $null = Set-WbRigPlacement -Process $play -Mask $rigPlan.PlayerMask -Role 'player' }
              Set-Content -LiteralPath $goFile -Value 'go' -Encoding ascii
              $players += $play
              $progress.Add("$(Get-Date -Format 'HH:mm:ss') player $i started for $title ($($s.seconds) s)")
            }
          }
          if ($l -match 'every session of the plan was recorded') { $allRecorded = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') all sessions recorded") }
          if ($l -match 'night pass planned') { $nightPlanned = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l") }
          if ($l -match 'night pass skipped') { $nightSkipped = Get-Date; $progress.Add("$(Get-Date -Format 'HH:mm:ss') $l") }
        }
        $lines = $ref.Value

        # One batch engine killed from outside, once an engine of the second or third session has run 20 s.
        if (-not $kill -and $startedTitles.Count -ge 2) {
          $tree = @(Get-WbProcessTree $emu.Id | ForEach-Object { [int]$_.ProcessId })
          $eng = @(Get-CimInstance Win32_Process -Filter "Name like 'wb-engine%'" -ErrorAction SilentlyContinue | Where-Object { $tree -contains [int]$_.ProcessId -and $_.CommandLine -match '\sbatch(\s|$)' })
          foreach ($e in $eng) {
            $age = ((Get-Date) - [datetime]$e.CreationDate).TotalSeconds
            if ($age -ge 20) {
              $kr = Invoke-WbNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/PID', "$($e.ProcessId)")
              $kill = [pscustomobject]@{ At = (Get-Date -Format 'HH:mm:ss'); Pid = $e.ProcessId; Name = $e.Name; AgeS = [math]::Round($age, 1); Exit = $kr.ExitCode }
              Add-Content -LiteralPath $queueLog -Value ("{0}`tSMOKE: killed engine pid {1} ({2}) after {3:n0} s, taskkill exit {4}" -f (Get-Date -Format 'HH:mm'), $e.ProcessId, $e.Name, $age, $kr.ExitCode) -Encoding UTF8
              $progress.Add("$(Get-Date -Format 'HH:mm:ss') killed engine pid $($e.ProcessId) $($e.Name) after $([math]::Round($age)) s")
              break
            }
          }
        }

        # End of the run: a short night pass, or the reason there is none.
        $stop = $false
        if ($nightPlanned -and ((Get-Date) - $nightPlanned).TotalMinutes -ge $NightMinutes) { $stop = $true; $progress.Add("$(Get-Date -Format 'HH:mm:ss') night pass time box of $NightMinutes min reached: stopping") }
        elseif ($nightSkipped -and ((Get-Date) - $nightSkipped).TotalSeconds -ge 30) { $stop = $true }
        elseif ($allRecorded -and ((Get-Date) - $allRecorded).TotalMinutes -ge 40) { $stop = $true; $progress.Add("$(Get-Date -Format 'HH:mm:ss') no night decision within 40 min of the last session: stopping") }
        elseif ((Get-Date) -gt $hardLimit) { $stop = $true; $progress.Add("$(Get-Date -Format 'HH:mm:ss') hard limit of 100 min: stopping") }
        if ($stop -and -not (Test-Path -LiteralPath $stopFile)) { Set-Content -LiteralPath $stopFile -Value 'stop' -Encoding ascii }

        if (((Get-Date) - $lastProgress).TotalSeconds -ge 120) {
          $lastProgress = Get-Date
          $tail = @($lastLine)
          Write-Host "$(Get-Date -Format 'HH:mm:ss') running $([int]((Get-Date) - $started).TotalMinutes) min; sessions started $($startedTitles.Count); last log: $($tail -join ' ')"
        }
        Start-Sleep -Milliseconds 1000
      }
    } finally {
      foreach ($p in $players) { if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
      if ($emu -and -not $emu.HasExited) {
        Set-Content -LiteralPath $stopFile -Value 'stop' -Encoding ascii
        if (-not $emu.WaitForExit(120000)) { $null = Invoke-WbNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/T', '/PID', "$($emu.Id)") }
      }
      foreach ($v in 'WB_GATE_DAY_PLAN', 'WB_GATE_DAY_STOP_FILE', 'WB_GATE_REPORT', 'WB_GATE_BACKUP', 'WB_GATE_QUEUE_LOG', 'WB_GATE_CLOCK_OFFSET_S', 'WB_GATE_NIGHT_WAIT_S', 'WB_GATE_PART_SECONDS', 'WB_GATE_ROOT', 'WB_GATE_DATA_DIR', 'WB_GATE_CHECK') {
        Remove-Item "Env:\$v" -ErrorAction SilentlyContinue
      }
    }
    Write-Utf8 (Join-Path $run 'script-notes.json') ((@{
          started = $started.ToString('o'); ended = (Get-Date).ToString('o'); emu_exit = $emu.ExitCode; clock_offset_s = $offset
          kill = $kill; progress = @($progress.ToArray()); profile = @{ tier = "$($d.tier)"; config = ($d.config.config | ConvertTo-Json -Compress); cpu_threads = $d.config.cpu_threads; process_while_recording = $d.process_while_recording }
          night_minutes = $NightMinutes; part_seconds = $PartSeconds; cable = @{ play = $playName; capture = $capName }
        } | ConvertTo-Json -Depth 6))
    $run
  }
  $runDir = @($result)[-1]
  Write-SmokeEvidence $runDir
} catch {
  $failed = $_
  Write-Host "FAIL: the smoke run did not complete: $($_.Exception.Message)"
  Write-Host $_.ScriptStackTrace
  exit 1
}
