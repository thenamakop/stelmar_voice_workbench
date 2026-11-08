<#
.SYNOPSIS
  Reproduces the white windows seen after launch (8 Nov 2026) and tells WebView2 crashes apart from
  other white windows.

.DESCRIPTION
  Modes ($Cycles each):
    immediate  force-kill the app's process tree, wait 0.5 s, relaunch (the old harness);
    clean      close the window, force-kill leftovers, wait until no WebView2 process for the app's
               user data folder remains (Stop-WbApp), relaunch.
  For each cycle: launch, wait for render, then sample the WebView2 processes for the app's
  --user-data-dir every 0.5 s for 8 s, then screenshot the window.
    crash-white  window white, the WebView2 browser process gone, and a new Crashpad dump
    other-white  window white otherwise (for example all processes alive)
  Every white screenshot is saved. -BrowserArgs sets WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS for this
  script's process and so for the app it launches (nothing global changes).

  Network on is fine. Do not use the mouse or keyboard while it runs (screenshots need the window
  on top). ASCII only; Windows PowerShell 5.1.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\repro-webview2-relaunch.ps1 -Mode clean -Cycles 40 -Tag baseline
  powershell -ExecutionPolicy Bypass -File .\scripts\repro-webview2-relaunch.ps1 -Mode clean -Cycles 40 -Tag gpu-off -BrowserArgs --disable-gpu
#>
[CmdletBinding()]
param(
  [string]$Exe,
  [string]$OutDir,
  [int]$Cycles = 20,
  [ValidateSet('both', 'immediate', 'clean')] [string]$Mode = 'both',
  [string]$Tag = 'repro',
  [string]$BrowserArgs,
  # Machine name written to the log instead of the computer name (the log goes through Redact.psm1).
  [string]$Machine = '%COMPUTERNAME%'
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Exe) { $Exe = Join-Path $here '..\target\release\stelmar-voice-workbench.exe' }
if (-not $OutDir) { $OutDir = Join-Path $here '..\docs\evidence\phase-0\webview2-repro' }
Import-Module (Join-Path $here 'lib\AppProcess.psm1') -Force
Import-Module (Join-Path $here 'lib\Redact.psm1') -Force
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
if ($BrowserArgs) { $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $BrowserArgs }

$log = New-Object System.Collections.Generic.List[string]
function Say([string]$line) { Write-Host $line; $log.Add($line) }
function Get-Dumps {
  $dir = Join-Path (Get-WbUserDataDir) 'Crashpad\reports'
  if (-not (Test-Path $dir)) { return @() }
  @(Get-ChildItem $dir -Filter '*.dmp' -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
}
function Format-Wv2($list) {
  if (-not @($list).Count) { return 'none' }
  (@($list) | Sort-Object Type, ProcessId | ForEach-Object { "$($_.Type):$($_.ProcessId)<-$($_.ParentProcessId)" }) -join ' '
}

Say "WebView2 relaunch repro ($Tag): STELMAR Voice Workbench release build"
Say "Date: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz')   Machine: $env:COMPUTERNAME   Windows $([Environment]::OSVersion.Version)"
Say "Exe: $Exe ($((Get-Item $Exe).LastWriteTime.ToString('yyyy-MM-dd HH:mm')))"
Say "User data folder: $(Get-WbUserDataDir) (exists: $(Test-Path (Get-WbUserDataDir)))"
Say "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: $(if ($env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS) { $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS } else { '(not set)' })"
Say "Timeline: seconds after render = WebView2 process count (b = browser alive, - = browser gone)"

$modes = if ($Mode -eq 'both') { @('immediate', 'clean') } else { @($Mode) }
$summary = @()
foreach ($m in $modes) {
  Say ""
  Say "== Mode: $m ($Cycles cycles)"
  [void](Stop-WbApp)
  $crashWhite = 0; $otherWhite = 0; $neverRendered = 0; $inherited = 0; $drainFail = 0; $newDumpsTotal = 0; $obscured = 0
  for ($i = 1; $i -le $Cycles; $i++) {
    if ($i -gt 1) {
      if ($m -eq 'immediate') { Stop-WbAppImmediate }
      elseif (-not (Stop-WbApp)) { $drainFail++ }
    }
    $dumpsBefore = Get-Dumps
    $run = Start-WbApp -Exe $Exe
    $timeline = @(); $state = 'never rendered'; $check = $null; $after = @()
    if ($null -ne $run.RenderMs) {
      for ($t = 1; $t -le 16; $t++) {
        Start-Sleep -Milliseconds 500
        $w = Get-WbWebView2
        $b = if (@($w | Where-Object { $_.Type -eq 'browser' }).Count) { 'b' } else { '-' }
        $timeline += ('{0:0.0}={1}{2}' -f ($t / 2), @($w).Count, $b)
      }
      $after = Get-WbWebView2
      if (-not $run.Process.HasExited -and $run.Handle -ne [IntPtr]::Zero) {
        $shot = Join-Path $OutDir ("{0}-{1}-cycle{2:00}.png" -f $Tag, $m, $i)
        $check = Get-WbWindowCheck $run.Handle $shot
        if ($check -and $check.Rendered) { Remove-Item $shot -ErrorAction SilentlyContinue }
      }
      $state = if ($check -and $check.Obscured) { 'obscured' } elseif ($check -and $check.Rendered) { 'ok' } else { 'WHITE' }
    } else { $neverRendered++ }
    $newDumps = @(Get-Dumps | Where-Object { $dumpsBefore -notcontains $_ })
    $newDumpsTotal += $newDumps.Count
    $browser = @($after | Where-Object { $_.Type -eq 'browser' })
    $isInherited = [bool](@($browser | Where-Object { $_.ParentProcessId -ne $run.Process.Id -or $_.Started -lt $run.LaunchedAt }).Count)
    if ($isInherited) { $inherited++ }
    $kind = ''
    if ($state -eq 'obscured') { $obscured++ }
    if ($state -eq 'WHITE') {
      if (-not $browser.Count -and $newDumps.Count) { $crashWhite++; $kind = ' crash-white' } else { $otherWhite++; $kind = ' other-white' }
    }
    Say ("  {0,2}: app {1}, render {2} ms, after 8 s {3}{4}, new dumps [{5}], inherited {6}, wv2 after [{7}], timeline {8}" -f `
        $i, $run.Process.Id, $run.RenderMs, $state, $kind, ($newDumps -join ' '), $isInherited, (Format-Wv2 $after), ($timeline -join ' '))
  }
  [void](Stop-WbApp)
  $line = "$Tag $m : crash-white $crashWhite/$Cycles, other-white $otherWhite/$Cycles, never rendered $neverRendered, new dumps $newDumpsTotal, inherited browser $inherited, drain failures $drainFail, not captured (window not in front) $obscured"
  Say "  $line"
  $summary += $line
}
Say ""
Say "== Summary"
foreach ($s in $summary) { Say "  $s" }
$out = Join-Path $OutDir "$Tag.txt"
Write-WbEvidence -Path $out -Lines $log -MachineLabel $Machine
Write-Host "Log: $out"
