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
