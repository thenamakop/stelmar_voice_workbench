<#
.SYNOPSIS
  Runs the six Phase 1 gates in order: panic, webview, emulator, kill, load, soak.

.DESCRIPTION
  Holds D:\Projects\stelmar-gate-request.flag for the whole sequence (removed in a finally block,
  whether the sequence finishes or stops), so Track B does not start new work. Each gate keeps
  its own rules (scripts/lib/GateRun.psm1): overlay tools closed, waits while Track B's busy lock
  exists, at least 14 GB of free RAM, D:\Projects\stelmar-gate.lock while it runs, free RAM at
  start and end in its evidence.

  The sequence STOPS (and does not pass the wait on) when a gate gave up waiting for Track B's
  busy lock after 30 minutes, or found less than 14 GB of RAM free: those need the user. A gate
  that runs and fails is recorded and the sequence goes on, so all six results are available.

  The log (one line per gate, with free RAM before it) goes to D:\STELMAR-Gate\run-all.log.
#>
[CmdletBinding()]
param(
  [string[]]$Gates = @('panic', 'webview', 'emulator', 'kill', 'load', 'soak'),
  [string]$Profile = 'emu-low',
  # Run even though the virtual cable is a default playback device (see recorder-gate.ps1).
  [switch]$AllowCableDefault,
  [string]$Log = 'D:\STELMAR-Gate\run-all.log',
  [string]$RequestFlag = 'D:\Projects\stelmar-gate-request.flag',
  # Where the evidence files go (default: each gate script's own default, docs\evidence\phase-1).
  [string]$OutDir = '',
  # Minutes for every listed recorder gate (0 = the gate's own length); for a short diagnostic run.
  [double]$Minutes = 0
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Import-Module (Join-Path $here '..\lib\PsEnv.psm1') -Force
Set-WbCleanPSModulePath
Set-WbInvariantCulture
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Log) | Out-Null

function Write-Log([string]$Line) {
  $stamped = "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') $Line"
  Add-Content -LiteralPath $Log -Value $stamped
  Write-Host $stamped
}

function Get-FreeRamGB { [math]::Round([double](Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB, 1) }

Set-Content -LiteralPath $Log -Value "phase 1 gate sequence ($($Gates -join ', ')) on $Profile"
Set-Content -LiteralPath $RequestFlag -Value "requested by=phase1 gate sequence`r`nstarted=$((Get-Date).ToString('o'))`r`npid=$PID`r`ngates=$($Gates -join ',')" -Encoding ascii
$stopReason = $null
try {
  foreach ($g in $Gates) {
    Write-Log "$g : starting (free RAM $(Get-FreeRamGB) GB)"
    $script = $(switch ($g) {
        'emulator' { Join-Path $here 'emulator-check.ps1' }
        'kill' { Join-Path $here 'kill-gate.ps1' }
        default { Join-Path $here 'recorder-gate.ps1' }
      })
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $script)
    if ($g -in @('panic', 'webview', 'load', 'soak', 'hashing')) { $argList += @('-Gate', $g, '-Profile', $Profile) }
    if ($g -eq 'kill') { $argList += @('-Profile', $Profile) }
    if ($OutDir) { $argList += @('-OutDir', $OutDir) }
    if ($Minutes -gt 0 -and $g -in @('panic', 'webview', 'load', 'soak', 'hashing')) { $argList += @('-Minutes', $Minutes) }
    if ($AllowCableDefault -and $g -ne 'emulator') { $argList += '-AllowCableDefault' }
    # The helper gives the child Windows PowerShell 5.1's own module path (a parent shell's
    # PowerShell 7 path makes Get-FileHash vanish) and collects stderr as text.
    $out = @(Invoke-WbWindowsPowerShell -ArgumentList $argList)
    $code = $LASTEXITCODE
    $text = $out -join "`n"
    $verdict = @($out | Where-Object { $_ -match '^(PASS|FAIL|INVALID): ' } | Select-Object -Last 1)
    if ($text -match 'Ask the user before running gate') {
      $stopReason = "$g : Track B's busy lock was held for more than 30 minutes"
    } elseif ($text -match 'GB of RAM free; gate .* needs at least') {
      $stopReason = "$g : less than 14 GB of RAM free"
    } elseif ($text -match 'The default playback device .* is CABLE Input') {
      $stopReason = "$g : the virtual cable is a default playback device"
    } elseif ($text -match 'Overlay tools are running') {
      $stopReason = "$g : overlay tools are running"
    } elseif ($text -match 'An engine build is running') {
      $stopReason = "$g : an engine build is running"
    }
    if ($stopReason) {
      Write-Log "$g : NOT RUN. $stopReason. Sequence stopped; ask the user."
      break
    }
    $summary = $(if ($verdict.Count) { $verdict[0] } else { "exit $code; last output: " + (($out | Select-Object -Last 3) -join ' | ') })
    Write-Log "$g : finished (exit $code): $summary (free RAM after $(Get-FreeRamGB) GB)"
  }
} finally {
  Remove-Item -LiteralPath $RequestFlag -Force -ErrorAction SilentlyContinue
  Write-Log "sequence $(if ($stopReason) { 'STOPPED' } else { 'finished' }); $RequestFlag removed"
}
if ($stopReason) { exit 2 }
