<#
.SYNOPSIS
  Phase 2 Night 2: speed tables A, B and C in order, one log, resumable.

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\gates\night2.ps1

.DESCRIPTION
  Runs scripts\gates\speed-tables.ps1 for table A (emu-low), then B (emu-mid), then C (native), each under the gate
  lock with the overlay, build, free-RAM and Track B rules. The request flag D:\Projects\stelmar-gate-request.flag is
  held for the whole sequence so Track B starts nothing new. Results are saved cell by cell in
  D:\STELMAR-Gate\speed\results-<table>.jsonl: if the run is stopped (power, sleep, a crash) start the same command
  again and it carries on with the first cell that is not saved.

  Before you start: close RTSS and MSI Afterburner, keep the laptop on mains power with the lid open, set the power plan
  to the one the earlier gates used, and do not use the computer; stop the display from sleeping. No network is needed.
  Log: D:\STELMAR-Gate\night2.log (one line per table with free RAM and the time).
  Evidence: docs\evidence\phase-2\speed-table-A.txt, -B.txt, -C.txt (rewritten after each table).
#>
[CmdletBinding()]
param(
  [string[]]$Tables = @('A', 'B', 'C'),
  [string]$Log = 'D:\STELMAR-Gate\night2.log',
  [string]$RequestFlag = 'D:\Projects\stelmar-gate-request.flag',
  [int]$Runs = 3
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Log) | Out-Null
function Write-Log([string]$Line) {
  $stamped = "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') $Line"
  Add-Content -LiteralPath $Log -Value $stamped
  Write-Host $stamped
}
function Get-FreeRamGB { [math]::Round([double](Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB, 1) }

Write-Log "night 2 starting (tables $($Tables -join ', ')); free RAM $(Get-FreeRamGB) GB"

# ---- preconditions: recorded in the log; any hard failure stops here and nothing starts ----
$repo = (Resolve-Path (Join-Path $here '..\..')).Path
Import-Module (Join-Path $repo 'scripts\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
Import-Module (Join-Path $repo 'scripts\lib\GateRun.psm1') -Force
$hard = @()
function Check([string]$What, [bool]$Ok, [string]$Detail) {
  Write-Log "precondition: $What : $(if ($Ok) { 'ok' } else { 'FAILED' })$(if ($Detail) { ' (' + $Detail + ')' })"
  if (-not $Ok) { $script:hard += $What }
}
$overlay = @(Get-WbOverlayTools)
Check 'overlay tools (RTSS, MSI Afterburner) closed' ($overlay.Count -eq 0) ($overlay -join ', ')
$builds = @(Get-WbRunningBuilds)
Check 'no engine build or compiler running, no build lock' ($builds.Count -eq 0) ($builds -join ', ')
$free = Get-WbFreeRamGB
Check 'at least 14 GB of RAM free' ($free -ge 14) "$free GB free"
Check 'no other gate holds the gate lock' (-not (Test-Path 'D:\Projects\stelmar-gate.lock')) 'D:\Projects\stelmar-gate.lock'
Check 'Track B busy lock absent' (-not (Test-Path 'D:\Projects\stelmar-eval-busy.lock')) 'D:\Projects\stelmar-eval-busy.lock'
$heavy = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match '^(wb-engine|ffmpeg|stelmar-voice-workbench|whisper)' -or ($_.ProcessName -eq 'python' -and $_.StartTime -gt (Get-Date).AddHours(-12) -and $_.CPU -gt 60) } | ForEach-Object { "$($_.ProcessName) pid $($_.Id)" })
Check 'no engine, decoder, app or busy Python process running' ($heavy.Count -eq 0) ($heavy -join ', ')
$bat = @(Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue)
$onBattery = ($bat.Count -gt 0 -and ($bat | Where-Object { $_.BatteryStatus -eq 1 }).Count -gt 0)
Check 'on mains power (not discharging)' (-not $onBattery) "battery status $(($bat | ForEach-Object { $_.BatteryStatus }) -join ',')"
Write-Log ("power plan: " + ((& powercfg /getactivescheme) -join ' '))
$tb = Join-Path $repo 'target\release\wb-testbench.exe'
if (Test-Path $tb) {
  $dev = Invoke-WbNative -FilePath $tb -ArgumentList @('devices')
  try { $d = ($dev.Output -join "`n") | ConvertFrom-Json; Write-Log "default playback device: console=$($d.default_roles.console); multimedia=$($d.default_roles.multimedia); communications=$($d.default_roles.communications) (information; the speed tables play and record nothing)"; $cable = @(Get-WbCableDefaultRoles -Testbench $tb); Check 'the virtual cable is not a default playback device' ($cable.Count -eq 0) ($cable -join ', ') } catch { Write-Log "default playback device: could not be read ($($_.Exception.Message))" }
}
foreach ($v in 'avx2', 'vulkan', 'cuda') { Check "engine $v built" (Test-Path (Join-Path $repo "src-tauri\binaries\wb-engine-$v-x86_64-pc-windows-msvc.exe")) '' }
foreach ($f in 'ggml-small.bin', 'ggml-medium.bin', 'ggml-large-v3-turbo-q5_0.bin', 'ggml-large-v3-q5_0.bin', 'ggml-silero-v6.2.0.bin') { Check "model $f present" (Test-Path (Join-Path $repo "models\$f")) '' }
$gitRev = Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $repo, 'rev-parse', '--short', 'HEAD')
$gitSt = Invoke-WbNative -FilePath 'git' -ArgumentList @('-C', $repo, 'status', '--porcelain')
Write-Log "source: commit $($gitRev.Output -join ''); uncommitted changes: $(@($gitSt.Output | Where-Object { $_ -notmatch 'permissions/autogenerated' }).Count) (autogenerated permission files excluded: build output, line endings only)"
if ($hard.Count) { Write-Log "night 2 NOT started: failed preconditions: $($hard -join '; ')"; exit 3 }

Set-Content -LiteralPath $RequestFlag -Value "requested by=night 2 speed tables`r`nstarted=$((Get-Date).ToString('o'))`r`npid=$PID" -Encoding ascii
$failed = @()
try {
  foreach ($t in $Tables) {
    Write-Log "table $t : starting (free RAM $(Get-FreeRamGB) GB)"
    $r = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $here 'speed-tables.ps1'), '-Tables', $t, '-Runs', "$Runs") -PassThru -Wait -NoNewWindow
    $code = $r.ExitCode
    Write-Log "table $t : finished with exit code $code (free RAM $(Get-FreeRamGB) GB)"
    if ($code -ne 0) {
      $failed += $t
      # The reasons that need you (Track B busy for 30 minutes, under 14 GB free, overlay tools, a build): stop, do not skip ahead.
      Write-Log "table $t : stopping the sequence; see the console output above and D:\STELMAR-Gate\speed\results-$t.jsonl"
      break
    }
  }
} finally {
  Remove-Item -LiteralPath $RequestFlag -Force -ErrorAction SilentlyContinue
}
if ($failed.Count) { Write-Log "night 2 stopped at table $($failed -join ', ')"; exit 1 }
Write-Log 'night 2 done'
