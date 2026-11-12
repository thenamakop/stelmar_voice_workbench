<#
  GateRun.psm1: shared rules for every timing-sensitive gate run (soak, load, kill, emulator
  checks, later benchmarks).

  1. Overlay tools: RivaTuner Statistics Server (RTSS.exe) and MSI Afterburner
     (MSIAfterburner.exe) must not run; their window hooks crashed WebView2 in Phase 0. The run
     stops with a message if either is running, and checks again at the end.
  2. Track B lock: D:\Projects\stelmar-gate.lock (gate name, start time, process id) exists for
     the whole run, so the Phase 5a worktree session pauses its heavy steps. It is removed in a
     finally block, so a failing gate cannot leave it behind.
  3. Evidence: every evidence file starts with the header lines from Get-WbGateHeader and is
     written through scripts/lib/Redact.psm1 (Write-WbEvidence).

  ASCII only; runs under Windows PowerShell 5.1.
#>

Set-StrictMode -Version 2

# Evidence is written through the redaction rules; do not depend on the caller having imported it.
Import-Module (Join-Path $PSScriptRoot 'Redact.psm1') -Force

$script:LockPath = 'D:\Projects\stelmar-gate.lock'
# Track B (the Phase 5a worktree session) holds this while it is mid-run; gates wait for it.
$script:TrackBBusyPath = 'D:\Projects\stelmar-eval-busy.lock'
# Free RAM a gate needs before it starts: the 11 GiB emulation job plus headroom.
$script:MinFreeRamGB = 14
$script:OverlayNames = @('RTSS', 'MSIAfterburner')
# Engine builds (cargo, CMake, the C++ and CUDA compilers) must not run while a timing-sensitive run
# is going: they take every core for minutes and would spoil a speed, load or soak measurement.
# scripts/build-engines.ps1 holds this lock for the whole build; the process names catch any other
# build, however it was started.
$script:BuildLockPath = 'D:\Projects\stelmar-build.lock'
$script:BuildProcessNames = @('cargo', 'rustc', 'cl', 'link', 'cmake', 'ninja', 'nvcc', 'glslc', 'MSBuild', 'cicc', 'ptxas', 'fatbinary', 'nvlink')
$script:Current = $null

function Get-WbOverlayTools {
  @(Get-Process -Name $script:OverlayNames -ErrorAction SilentlyContinue | ForEach-Object { "$($_.ProcessName).exe (pid $($_.Id))" })
}

function Assert-WbNoOverlayTools {
  $running = @(Get-WbOverlayTools)
  if ($running.Count -gt 0) {
    throw ("Overlay tools are running: " + ($running -join ', ') + ". Close RivaTuner Statistics Server and MSI Afterburner (including their tray icons), then run the gate again.")
  }
}

# Engine builds that are running now: the build lock and any compiler or build-tool process.
function Get-WbRunningBuilds {
  param([string]$BuildLockPath = $script:BuildLockPath, [string[]]$ProcessNames = $script:BuildProcessNames)
  $found = @()
  if (Test-Path -LiteralPath $BuildLockPath) { $found += "$BuildLockPath exists" }
  $found += @(Get-Process -Name $ProcessNames -ErrorAction SilentlyContinue | ForEach-Object { "$($_.ProcessName).exe (pid $($_.Id))" })
  $found
}

function Assert-WbNoBuilds {
  param([string]$BuildLockPath = $script:BuildLockPath, [string[]]$ProcessNames = $script:BuildProcessNames)
  $running = @(Get-WbRunningBuilds -BuildLockPath $BuildLockPath -ProcessNames $ProcessNames)
  if ($running.Count -gt 0) {
    throw ("An engine build is running: " + ($running -join ', ') + ". Nothing timing-sensitive may run while a build is going; wait for it to finish, then run the gate again.")
  }
}

function Get-WbFreeRamGB {
  $os = Get-CimInstance Win32_OperatingSystem
  [math]::Round([double]$os.FreePhysicalMemory / 1MB, 1)
}

# Runs $Body with the overlay check, the Track B busy-lock wait, the 14 GB RAM check and the gate
# lock (removed in a finally block). Returns whatever $Body returns; an exception from $Body is
# recorded in the run's Error and rethrown.
function Invoke-WbGate {
  param(
    [Parameter(Mandatory = $true)] [string]$Name,
    [Parameter(Mandatory = $true)] [scriptblock]$Body,
    [string]$LockPath = $script:LockPath,
    [int]$TrackBWaitMinutes = 30,
    [string]$TrackBBusyPath = $script:TrackBBusyPath,
    [int]$TrackBPollSeconds = 30,
    [double]$MinFreeRamGB = $script:MinFreeRamGB,
    [string]$BuildLockPath = $script:BuildLockPath,
    [string[]]$BuildProcessNames = $script:BuildProcessNames
  )
  Assert-WbNoOverlayTools
  Assert-WbNoBuilds -BuildLockPath $BuildLockPath -ProcessNames $BuildProcessNames
  # Track B mid-run: wait (check every 30 s) up to 30 minutes, then stop and ask.
  $waitStart = Get-Date
  $waited = $false
  while (Test-Path -LiteralPath $TrackBBusyPath) {
    $waited = $true
    if (((Get-Date) - $waitStart).TotalMinutes -ge $TrackBWaitMinutes) {
      $busy = Get-Content -LiteralPath $TrackBBusyPath -Raw -ErrorAction SilentlyContinue
      throw "Track B has held $TrackBBusyPath for over $TrackBWaitMinutes minutes:`n$busy`nAsk the user before running gate $Name."
    }
    Write-Host "$(Get-Date -Format 'HH:mm:ss') waiting: Track B holds $TrackBBusyPath"
    Start-Sleep -Seconds $TrackBPollSeconds
  }
  $waitedMin = [math]::Round(((Get-Date) - $waitStart).TotalMinutes, 1)
  $trackBWait = $(if ($waited) { "waited $waitedMin min for $TrackBBusyPath to clear" } else { "$TrackBBusyPath absent at start" })
  $freeStart = Get-WbFreeRamGB
  if ($freeStart -lt $MinFreeRamGB) {
    throw "Only $freeStart GB of RAM free; gate $Name needs at least $MinFreeRamGB GB (the 11 GiB emulation job plus headroom). Close other programs and run it again."
  }
  if (Test-Path -LiteralPath $LockPath) {
    $held = (Get-Content -LiteralPath $LockPath -Raw)
    throw "Another gate holds $LockPath :`n$held`nWait for it to finish (or delete the file if no gate is running)."
  }
  $started = Get-Date
  $lockText = "gate=$Name`r`nstarted=$($started.ToString('o'))`r`npid=$PID`r`n"
  Set-Content -LiteralPath $LockPath -Value $lockText -Encoding ascii
  $script:Current = [pscustomobject]@{ Name = $Name; Started = $started; LockPath = $LockPath; OverlayAtEnd = $null; BuildsAtEnd = $null; BuildLockPath = $BuildLockPath; TrackBWait = $trackBWait; FreeRamStart = $freeStart; FreeRamEnd = $null; Error = $null; Note = $null }
  try {
    & $Body
  } catch {
    # Keep the full exception (type, message, script stack) for the evidence file, then let the
    # gate script handle it (it writes FAIL evidence with Write-WbGateCrashEvidence).
    $script:Current.Error = ($_.Exception.GetType().FullName + ': ' + $_.Exception.Message + "`r`n" + $_.ScriptStackTrace + "`r`n" + ($_ | Out-String))
    throw
  } finally {
    $script:Current.OverlayAtEnd = @(Get-WbOverlayTools)
    $script:Current.BuildsAtEnd = @(Get-WbRunningBuilds -BuildLockPath $BuildLockPath -ProcessNames $BuildProcessNames)
    $script:Current.FreeRamEnd = Get-WbFreeRamGB
    Remove-Item -LiteralPath $LockPath -Force -ErrorAction SilentlyContinue
  }
}

# The repository root, found from this module's own location (scripts\lib), never from the
# current folder: the gates are started from any folder.
function Get-WbRepoRoot {
  (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
}

# Runs a native command and returns its exit code, stdout lines and stderr lines. Under Windows
# PowerShell 5.1 a redirected stderr line (2>$null, 2>&1) becomes an error record, and with
# $ErrorActionPreference = 'Stop' that is a terminating NativeCommandError that ends the gate (a
# 3-hour run ended at its evidence step on "fatal: not a git repository"). Here the preference is
# relaxed for the call and stderr is collected as text, so the caller decides what a failure means.
# A command that does not exist comes back as exit code -1 with the reason in Error.
function Invoke-WbNative {
  param(
    [Parameter(Mandatory = $true)] [string]$FilePath,
    [string[]]$ArgumentList = @()
  )
  $prev = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  $all = @()
  $code = -1
  try {
    $all = @(& $FilePath @ArgumentList 2>&1)
    $code = $LASTEXITCODE
  } catch {
    $all = @($_)
    $code = -1
  } finally {
    $ErrorActionPreference = $prev
  }
  $out = New-Object System.Collections.Generic.List[string]
  $err = New-Object System.Collections.Generic.List[string]
  foreach ($x in $all) {
    if ($x -is [System.Management.Automation.ErrorRecord]) { $err.Add("$x") } else { $out.Add("$x") }
  }
  [pscustomobject]@{ ExitCode = $code; Output = @($out); Error = @($err) }
}

# Header lines for an evidence file of the current (or just finished) gate run.
function Get-WbGateHeader {
  param([string]$Machine = 'Legion', [string]$Profile = '', [string[]]$Builds = @())
  $c = $script:Current
  $lines = @()
  # Which code produced this evidence: commit, uncommitted changes, and the executables' hashes.
  # git -C the repository root, so it works from any folder. If git fails the evidence still gets
  # written (the build hashes identify the code) and says so, instead of losing a finished run.
  $root = Get-WbRepoRoot
  $rev = Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $root, 'rev-parse', '--short', 'HEAD')
  if ($rev.ExitCode -eq 0 -and $rev.Output.Count -gt 0) {
    $st = Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $root, 'status', '--porcelain')
    $dirty = ($st.ExitCode -eq 0 -and $st.Output.Count -gt 0)
    $lines += "source: commit $($rev.Output[0])$(if ($dirty) { ' with uncommitted or untracked changes (the build hashes below identify the code)' } else { '' })"
  } else {
    $why = (@($rev.Error) + @($rev.Output)) -join ' '
    $lines += "source: commit unknown (git failed in the repository: $why); the build hashes below identify the code"
  }
  foreach ($b in $Builds) {
    if ($b -and (Test-Path -LiteralPath $b)) {
      $h = (Get-FileHash -LiteralPath $b -Algorithm SHA256).Hash.ToLowerInvariant()
      $item = Get-Item -LiteralPath $b
      $lines += "build: $($item.Name) sha256 $h (built $($item.LastWriteTime.ToString('yyyy-MM-dd HH:mm')))"
    }
  }
  if ($c) {
    $lines += "gate: $($c.Name)"
    $lines += "started: $($c.Started.ToString('yyyy-MM-dd HH:mm:ss zzz'))"
    if ($c.Note) { $lines += "note: $($c.Note)" }
  }
  $lines += "machine: $Machine"
  if ($Profile) { $lines += "emulation profile: $Profile" }
  # A re-evaluated run (-FromRun) did not take these measurements now: say so instead of claiming them.
  $fromRun = ($c -and $c.Name -like '*re-evaluated*')
  if ($fromRun) {
    $lines += 'overlay tools closed: checked by the original run before it started (the gate refuses to start otherwise); not saved, not re-checked'
  } else {
    $lines += 'overlay tools closed: yes (RTSS.exe and MSIAfterburner.exe not running when the gate started)'
  }
  if ($c -and $null -ne $c.OverlayAtEnd) {
    if ($c.OverlayAtEnd.Count -gt 0) {
      $lines += ('overlay tools at the end: RUNNING (' + ($c.OverlayAtEnd -join ', ') + ') - result not valid')
    } else {
      $lines += 'overlay tools at the end: none running'
    }
  }
  if ($fromRun) {
    $lines += 'engine builds: checked by the original run before it started (the gate refuses to start otherwise); not saved, not re-checked'
  } else {
    $lines += "engine builds: none running when the gate started ($($script:BuildLockPath) absent; no $(($script:BuildProcessNames | ForEach-Object { "$_.exe" }) -join ', ') process)"
  }
  if ($c -and $null -ne $c.BuildsAtEnd) {
    if ($c.BuildsAtEnd.Count -gt 0) {
      $lines += ('engine builds at the end: RUNNING (' + ($c.BuildsAtEnd -join ', ') + ') - timing may be spoiled; result not valid')
    } else {
      $lines += 'engine builds at the end: none running'
    }
  }
  if ($fromRun) {
    $lines += "Track B paused: not re-evaluated (the original run's gate code takes $($script:LockPath); whether it was held is not in the saved data)"
  } else {
    $lines += "Track B paused: $($script:LockPath) held for the whole run (removed in a finally block)"
  }
  if ($c) {
    if (-not $fromRun) { $lines += "Track B busy lock: $($c.TrackBWait)" }
    $lines += "free RAM: $($c.FreeRamStart) GB at start (needs $($script:MinFreeRamGB) GB), $(if ($null -ne $c.FreeRamEnd) { "$($c.FreeRamEnd) GB" } else { 'not measured' }) at end"
  }
  $lines
}

# Sorts the app recording's discontinuities (wb-testbench analyse) against the independent
# reference capture of the same cable (wb-testbench capture):
#   Excused: at most ONE start-up repeat that the reference also has at the same source position
#            (+-50 ms, same jump): a repeat (negative jump) of at most 25 ms (one device period)
#            within the first 3 s of playback. Seen twice on the Legion (0.4 s and 2.6 s after
#            playback began) with no xrun or overrun on the capture side.
#   Chain:   any other break the reference also has. The reference shares the audio engine, the
#            cable driver and cpal with the app, so these are capture-chain breaks: they FAIL
#            and are listed for investigation.
#   Own:     breaks only in the app's recording (the recorder's own, or restarts in the kill test).
function Split-WbDiscontinuities {
  param($App, $Reference)
  $ref = @($Reference.discontinuities)
  $excused = @(); $chain = @(); $own = @()
  foreach ($d in @($App.discontinuities)) {
    $match = $ref | Where-Object { [math]::Abs([double]$_.source_ms - [double]$d.source_ms) -le 50 -and [int64]$_.source_jump_frames -eq [int64]$d.source_jump_frames } | Select-Object -First 1
    if (-not $match) { $own += $d; continue }
    $startupRepeat = ([double]$d.source_jump_ms -lt 0 -and [double]$d.source_jump_ms -ge -25 -and [double]$d.source_ms -le 3000)
    if ($startupRepeat -and $excused.Count -eq 0) { $excused += $d } else { $chain += $d }
  }
  [pscustomobject]@{ Excused = $excused; Chain = $chain; Own = $own }
}

# Source positions (ms) of the app recording's breaks (unmatched stretches and discontinuities), split
# into those the reference capture shows too (within 100 ms of one of its own: Shared, upstream) and
# those only the app shows (AppOnly). The evidence labels them by this split: a break the reference
# does not have is the app's, whatever the player's callbacks did.
function Split-WbBreakSources {
  param($App, $Reference)
  $refSrc = @()
  if ($Reference) {
    $refSrc = @(@($Reference.unmatched_ranges) | ForEach-Object { [double]$_.source_ms }) + @(@($Reference.discontinuities) | ForEach-Object { [double]$_.source_ms })
  }
  $appSrc = @(@($App.unmatched_ranges) | ForEach-Object { [double]$_.source_ms }) + @(@($App.discontinuities) | ForEach-Object { [double]$_.source_ms })
  $shared = @(); $only = @()
  foreach ($s in @($appSrc | Sort-Object -Unique)) {
    if (@($refSrc | Where-Object { [math]::Abs($_ - $s) -le 100 }).Count -gt 0) { $shared += $s } else { $only += $s }
  }
  [pscustomobject]@{ Shared = @($shared); AppOnly = @($only) }
}
# Milliseconds of the app recording's unmatched stretches (by source position) that lie outside
# +-100 ms of every discontinuity in $Near. Only the overlap with those windows is explained: a
# long stretch that merely starts next to a break still counts, beyond the window, as unexplained.
function Get-WbUnexplainedUnmatchedMs {
  param($App, $Near = @())
  $total = 0.0
  foreach ($u in @($App.unmatched_ranges)) {
    $start = [double]$u.source_ms
    $end = $start + [double]$u.ms
    # Windows clipped to this range, sorted, then merged so overlaps are not counted twice.
    $wins = @($Near | ForEach-Object {
        $lo = [math]::Max($start, [double]$_.source_ms - 100)
        $hi = [math]::Min($end, [double]$_.source_ms + 100)
        if ($hi -gt $lo) { ,@($lo, $hi) }
      } | Sort-Object { $_[0] })
    $explained = 0.0
    $curLo = $null; $curHi = $null
    foreach ($w in $wins) {
      if ($null -eq $curLo) { $curLo = $w[0]; $curHi = $w[1]; continue }
      if ($w[0] -le $curHi) { $curHi = [math]::Max($curHi, $w[1]) }
      else { $explained += $curHi - $curLo; $curLo = $w[0]; $curHi = $w[1] }
    }
    if ($null -ne $curLo) { $explained += $curHi - $curLo }
    $total += ([double]$u.ms - $explained)
  }
  [math]::Round($total, 3)
}

# Quotes one argument for Start-Process -ArgumentList (Windows PowerShell 5.1 does not).
function ConvertTo-WbArg([string]$Value) {
  if ($Value -match '[\s"]') { '"' + ($Value -replace '"', '\"') + '"' } else { $Value }
}

# Sets the context an evidence header describes, for a run that already happened (a gate whose
# evaluation is redone from its saved recording and reports instead of being run again).
function Set-WbGateContext {
  param(
    [Parameter(Mandatory = $true)] [string]$Name,
    [Parameter(Mandatory = $true)] [datetime]$Started,
    [double]$FreeRamStart = 0,
    [double]$FreeRamEnd = 0,
    [string]$TrackBWait = 'recorded by the original run',
    [string]$Note = ''
  )
  $script:Current = [pscustomobject]@{
    Name = $Name; Started = $Started; LockPath = $script:LockPath; OverlayAtEnd = $null
    TrackBWait = $TrackBWait; FreeRamStart = $FreeRamStart; FreeRamEnd = $FreeRamEnd; Error = $null; Note = $Note
  }
}

# Runs `wb-testbench analyse` and returns what it wrote to standard error (empty when it ran
# cleanly), also kept in <Report>.err. The report itself is only written when a signal was found.
function Invoke-WbAnalyse {
  param(
    [Parameter(Mandatory = $true)] [string]$Testbench,
    [Parameter(Mandatory = $true)] [string]$Session,
    [Parameter(Mandatory = $true)] [string]$Source,
    [Parameter(Mandatory = $true)] [string]$Report
  )
  Remove-Item -LiteralPath $Report, "$Report.err", "$Report.out" -Force -ErrorAction SilentlyContinue
  $argList = @('analyse', '--session', (ConvertTo-WbArg $Session), '--source', (ConvertTo-WbArg $Source), '--report', (ConvertTo-WbArg $Report))
  $p = Start-Process -FilePath $Testbench -ArgumentList $argList -Wait -PassThru -WindowStyle Hidden -RedirectStandardOutput "$Report.out" -RedirectStandardError "$Report.err"
  $text = ''
  if (Test-Path -LiteralPath "$Report.err") { $text = (Get-Content -LiteralPath "$Report.err" -Raw) }
  if ($null -eq $text) { $text = '' }
  $text = $text.Trim()
  if ($p.ExitCode -ne 0 -and -not $text) { $text = "wb-testbench analyse exited with code $($p.ExitCode)" }
  $text
}

# Windows roles (console, multimedia, communications) whose default playback device is the
# virtual cable (name contains $Cable). While it is, every sound the machine makes (notification
# chimes, any app on the default device) is played into the test signal: seen in Phase 1, where a
# two-tone chime reached the cable five times in 3 hours and failed the load and soak gates.
#
# -Cable is a list of names; a role matches when its default device's name contains any of them.
# The default list holds the product name too: after the 12 Nov 2026 restart the cable's playback
# endpoint is called "Speakers (3- VB-Audio Virtual Cable)", which contains no "CABLE Input", and
# the old check let the cable stay the default device.
function Get-WbCableDefaultRolesFromDevices {
  param($Devices, [string[]]$Cable = @('CABLE Input', 'VB-Audio Virtual Cable'))
  if ($null -eq $Devices -or -not (Test-WbHasProperty $Devices 'default_roles') -or $null -eq $Devices.default_roles) { return @() }
  $roles = @()
  foreach ($role in 'console', 'multimedia', 'communications') {
    $name = [string]$Devices.default_roles.$role
    if (-not $name) { continue }
    foreach ($c in $Cable) {
      if ($c -and $name.IndexOf($c, [StringComparison]::OrdinalIgnoreCase) -ge 0) { $roles += $role; break }
    }
  }
  $roles
}

function Get-WbCableDefaultRoles {
  param([Parameter(Mandatory = $true)] [string]$Testbench, [string[]]$Cable = @('CABLE Input', 'VB-Audio Virtual Cable'))
  $res = Invoke-WbNative -FilePath $Testbench -ArgumentList @('devices')
  if ($res.ExitCode -ne 0 -or $res.Output.Count -eq 0) { return @() }
  Get-WbCableDefaultRolesFromDevices -Devices (($res.Output -join "`n") | ConvertFrom-Json) -Cable $Cable
}

# Copies a List (or any collection) to an array. In Windows PowerShell 5.1, @($list) throws
# "Argument types do not match" for a List[object] that holds objects: that crashed the kill gate
# when it built its result (Rounds = @($rounds)), at the end of an 11-minute run, twice. Always
# use this for the Lists the gates fill; wrap the call in @() to keep a one-element result an array.
function ConvertTo-WbArray {
  param($List)
  if ($null -eq $List) { return }
  if ($List -is [System.Collections.IList]) { foreach ($item in $List.ToArray()) { $item } } else { foreach ($item in $List) { $item } }
}

# The driver-xrun rule (approved 9 Nov 2026). Each stream start while audio is already flowing counts one xrun
# in cpal (which packet Windows flags is not proven): every app start in the kill test counts exactly one, in its
# first second, with nothing lost around it. So:
#   Allowed: ONE driver xrun per run (per stream start or restart), no later than $WindowMs after
#            that run's first audio, and only when the sample comparison shows zero loss and
#            duplication ($LossFree: no break of the recording other than the restarts, no silent
#            gap, no unmatched audio). It is reported, not gated.
#   FAIL:    more than one xrun in a run, any xrun after the window, any xrun together with loss
#            or duplication, and any ring overrun or dropped sample at all.
# Two kinds of `gap` event carry the xruns, and the recorder writes both:
#   cause=xrun n=.. callback=.. frames=.. audio_ms=.. qpc=.. qpc_hz=..   (since 10 Nov 2026) one per xrun, written when
#            the error callback fires: the callback count, the audio position and the QPC tick are exact.
#   overrun_samples+N xruns+N dropped_samples+N (totals ..)              the counters, seen to move about a second
#            later (the audio time is an upper bound). The same xruns again, so a run that has `cause=xrun` events
#            counts those and uses the counter events only for the overrun and dropped-sample checks, plus any
#            xruns they count beyond the `cause=xrun` events (the recorder caps how many it writes to session.json).
#            A session from before 10 Nov has only the counter events and is read as it always was.
# `cause=position_break` events (the audio clock jumped) are listed in PositionBreaks and are not part of the xrun rule.
function Get-WbXrunVerdict {
  param(
    [Parameter(Mandatory = $true)] [string]$SessionDir,
    [bool]$LossFree = $true,
    [int]$WindowMs = 2000
  )
  $meta = (Get-Content -LiteralPath (Join-Path $SessionDir 'session.json') -Raw) | ConvertFrom-Json
  $problems = @()
  $positionBreaks = @()
  $runs = New-Object System.Collections.Generic.List[object]
  $cur = $null
  foreach ($e in @($meta.events)) {
    if ($e.kind -eq 'audio_started') {
      $cur = [pscustomobject]@{ Start = [double]$e.audio_ms; Cause = (New-Object System.Collections.Generic.List[object]); Counter = (New-Object System.Collections.Generic.List[object]) }
      $runs.Add($cur)
      continue
    }
    if ($e.kind -ne 'gap') { continue }
    if ($null -eq $cur) {
      # A gap before any audio_started: measured from nothing, so it can never be inside a window.
      $cur = [pscustomobject]@{ Start = $null; Cause = (New-Object System.Collections.Generic.List[object]); Counter = (New-Object System.Collections.Generic.List[object]) }
      $runs.Add($cur)
    }
    $detail = $(if (Test-WbHasProperty $e 'detail') { [string]$e.detail } else { '' })
    if ($detail -match 'overrun_samples\+(\d+) xruns\+(\d+) dropped_samples\+(\d+)') {
      $overrun = [int64]$Matches[1]; $xr = [int64]$Matches[2]; $dropped = [int64]$Matches[3]
      if ($overrun -gt 0 -or $dropped -gt 0) { $problems += "$overrun overrun and $dropped dropped samples at audio $($e.audio_ms) ms" }
      if ($xr -gt 0) { $cur.Counter.Add([pscustomobject]@{ AudioMs = [double]$e.audio_ms; Xruns = $xr }) }
    }
    elseif ($detail -match '^cause=xrun n=(\d+) callback=(\d+) frames=(\d+) audio_ms=(\d+) qpc=(\d+) qpc_hz=(\d+)') {
      $cur.Cause.Add([pscustomobject]@{ N = [int64]$Matches[1]; Callback = [int64]$Matches[2]; Frames = [int64]$Matches[3]; AudioMs = [double]$Matches[4]; Qpc = [int64]$Matches[5]; QpcHz = [int64]$Matches[6] })
    }
    elseif ($detail -match '^cause=position_break callback=(\d+) frames=(\d+) audio_ms=(\d+)') {
      $positionBreaks += [pscustomobject]@{ AudioMs = [double]$Matches[3]; Callback = [int64]$Matches[1]; Frames = [int64]$Matches[2] }
    }
    else {
      $problems += "a gap event at audio $($e.audio_ms) ms could not be read"
    }
  }
  $total = 0
  $excused = @()
  foreach ($run in $runs) {
    # Every xrun of this run as (audio time, how exact, callback, qpc).
    $items = @()
    foreach ($c in $run.Cause) { $items += [pscustomobject]@{ AudioMs = $c.AudioMs; Exact = $true; Callback = $c.Callback; Qpc = $c.Qpc } }
    $counted = [int64]0
    foreach ($c in $run.Counter) { $counted += $c.Xruns }
    $extra = $counted - $run.Cause.Count
    if ($extra -gt 0) {
      # xruns the counters saw that no `cause=xrun` event describes (an older session, or the event cap): place them at the counter's time.
      foreach ($c in $run.Counter) { for ($i = 0; $i -lt $c.Xruns -and $extra -gt 0; $i++) { $items += [pscustomobject]@{ AudioMs = $c.AudioMs; Exact = $false; Callback = $null; Qpc = $null }; $extra-- } }
    }
    $n = 0
    foreach ($it in $items) {
      $n++; $total++
      $offset = $(if ($null -ne $run.Start) { $it.AudioMs - $run.Start } else { [double]::PositiveInfinity })
      if ($n -gt 1) { $problems += "more than one driver xrun in one run (audio $($it.AudioMs) ms)" }
      elseif ($offset -gt $WindowMs) { $problems += "a driver xrun $([math]::Round($offset)) ms after the stream started (limit $WindowMs ms), audio $($it.AudioMs) ms" }
      elseif (-not $LossFree) { $problems += "a driver xrun with loss or duplication in the recording, audio $($it.AudioMs) ms" }
      else { $excused += [pscustomobject]@{ AudioMs = $it.AudioMs; OffsetMs = $offset; Exact = $it.Exact; Callback = $it.Callback; Qpc = $it.Qpc } }
    }
  }
  [pscustomobject]@{ Pass = ($problems.Count -eq 0); TotalXruns = $total; Excused = @($excused); Problems = @($problems); PositionBreaks = @($positionBreaks) }
}

# True when $Obj has a member called $Name (safe under strict mode, also for an object with no
# members at all, where PSObject.Properties.Name itself would throw).
function Test-WbHasProperty {
  param($Obj, [string]$Name)
  if ($null -eq $Obj) { return $false }
  foreach ($p in $Obj.PSObject.Properties) { if ($p.Name -eq $Name) { return $true } }
  $false
}

# Names of the required members of a gate result that are missing or null. A gate whose analysis
# step failed (for example the analyser found no signal) has $null there, and reading a property of
# $null under strict mode throws; the gate reports "no verdict data" with these names instead.
function Test-WbGateData {
  param($Result, [string[]]$Required)
  $missing = @()
  foreach ($name in $Required) {
    $has = (Test-WbHasProperty $Result $name) -and $null -ne $Result.$name
    if (-not $has) { $missing += $name }
  }
  $missing
}

# Writes a FAIL evidence file for a gate that did not complete (an exception, or data missing),
# so every gate ends with a verdict and the reason, through the evidence redaction rules.
function Write-WbGateCrashEvidence {
  param(
    [Parameter(Mandatory = $true)] [string]$Path,
    [Parameter(Mandatory = $true)] [string]$Reason,
    [string]$Detail = '',
    [string]$Machine = 'Legion',
    [string]$Profile = '',
    [string[]]$Builds = @(),
    [string[]]$Extra = @()
  )
  $lines = @(Get-WbGateHeader -Machine $Machine -Profile $Profile -Builds $Builds)
  $lines += 'verdict: FAIL (the gate did not complete)'
  $lines += ''
  $lines += "reason: $Reason"
  $lines += $Extra
  if ($Detail) {
    $lines += ''
    $lines += 'details:'
    $lines += ($Detail -split "`r?`n" | ForEach-Object { "  $_" })
  }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
  Write-WbEvidence -Path $Path -Lines $lines -MachineLabel $Machine
}

Export-ModuleMember -Function Get-WbOverlayTools, Get-WbRunningBuilds, Assert-WbNoBuilds, Assert-WbNoOverlayTools, Invoke-WbGate, Get-WbGateHeader, ConvertTo-WbArg, Split-WbDiscontinuities, Split-WbBreakSources, Get-WbUnexplainedUnmatchedMs, Test-WbGateData, Write-WbGateCrashEvidence, Get-WbFreeRamGB, Set-WbGateContext, Invoke-WbAnalyse, Test-WbHasProperty, Get-WbCableDefaultRoles, Get-WbCableDefaultRolesFromDevices, ConvertTo-WbArray, Get-WbXrunVerdict, Invoke-WbNative, Get-WbRepoRoot
