<#
  AppProcess.psm1: start, find, screenshot and stop the STELMAR Voice Workbench release build and
  the WebView2 processes it uses. Shared by scripts/offline-check.ps1 and
  scripts/repro-webview2-relaunch.ps1 (and, later, the Phase 8 offline proof).

  WebView2 runs one browser process per user data folder. A new app instance attaches to a browser
  process that is still running for the same folder, even one that belongs to an instance being
  killed. So WebView2 processes are found by the --user-data-dir on their command line, not by the
  app's process tree, and Stop-WbApp waits until none remain.

  ASCII only; runs under Windows PowerShell 5.1.
#>

Set-StrictMode -Version 2

Add-Type -AssemblyName System.Drawing
if (-not ('WbWin32v2' -as [type])) {
  Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WbWin32v2 {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
}
'@
}
[void][WbWin32v2]::SetProcessDPIAware()

$script:AppProcessName = 'stelmar-voice-workbench'
$script:Identifier = 'in.edu.bmu.stelmar.voiceworkbench'

function Get-WbUserDataDir {
  Join-Path $env:LOCALAPPDATA "$script:Identifier\EBWebView"
}

function Get-WbProcessTree([int]$RootId) {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
  $ids = New-Object 'System.Collections.Generic.HashSet[int]'
  [void]$ids.Add($RootId)
  do {
    $added = $false
    foreach ($p in $all) {
      if ($ids.Contains([int]$p.ParentProcessId) -and -not $ids.Contains([int]$p.ProcessId)) {
        [void]$ids.Add([int]$p.ProcessId); $added = $true
      }
    }
  } while ($added)
  @($all | Where-Object { $ids.Contains([int]$_.ProcessId) })
}

# Every msedgewebview2.exe whose command line names this app's user data folder.
function Get-WbWebView2 {
  $dir = (Get-WbUserDataDir).ToLowerInvariant()
  @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -ErrorAction SilentlyContinue |
      Where-Object { $_.CommandLine -and $_.CommandLine.ToLowerInvariant().Contains($dir) } |
      ForEach-Object {
        $type = if ($_.CommandLine -match '--type=([a-z-]+)') { $Matches[1] } else { 'browser' }
        [pscustomobject]@{
          ProcessId       = [int]$_.ProcessId
          ParentProcessId = [int]$_.ParentProcessId
          Type            = $type
          Started         = $_.CreationDate
        }
      })
}

# Old behaviour, kept for the repro: force-kill the app's process tree and relaunch at once.
function Stop-WbAppImmediate {
  Get-Process $script:AppProcessName -ErrorAction SilentlyContinue | ForEach-Object {
    foreach ($p in (Get-WbProcessTree $_.Id)) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }
  }
  Start-Sleep -Milliseconds 500
}

# Clean stop: close the window, wait, force-kill what is left (app and every WebView2 process for
# this user data folder), then wait until none remain. Returns $true when everything is gone.
function Stop-WbApp {
  param([int]$GraceSeconds = 5, [int]$DrainSeconds = 10)
  $apps = @(Get-Process $script:AppProcessName -ErrorAction SilentlyContinue)
  foreach ($a in $apps) { if ($a.MainWindowHandle -ne [IntPtr]::Zero) { [void]$a.CloseMainWindow() } }
  $deadline = (Get-Date).AddSeconds($GraceSeconds)
  while ((Get-Date) -lt $deadline -and @(Get-Process $script:AppProcessName -ErrorAction SilentlyContinue).Count) {
    Start-Sleep -Milliseconds 200
  }
  Get-Process $script:AppProcessName -ErrorAction SilentlyContinue | ForEach-Object {
    foreach ($p in (Get-WbProcessTree $_.Id)) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }
  }
  foreach ($w in (Get-WbWebView2)) { Stop-Process -Id $w.ProcessId -Force -ErrorAction SilentlyContinue }
  $deadline = (Get-Date).AddSeconds($DrainSeconds)
  while ((Get-Date) -lt $deadline) {
    if (-not @(Get-Process $script:AppProcessName -ErrorAction SilentlyContinue).Count -and -not @(Get-WbWebView2).Count) { return $true }
    Start-Sleep -Milliseconds 200
  }
  return $false
}

# The window's visible bounds. GetWindowRect on Windows 10 and 11 includes the invisible resize
# border (about 7 logical px on the left, right and bottom), so a capture from it shows a strip of
# whatever window is behind. DWMWA_EXTENDED_FRAME_BOUNDS (9) is the frame the user sees.
function Get-WbWindowBounds([IntPtr]$Handle) {
  $r = New-Object WbWin32v2+RECT
  $size = [System.Runtime.InteropServices.Marshal]::SizeOf([type][WbWin32v2+RECT])
  if ([WbWin32v2]::DwmGetWindowAttribute($Handle, 9, [ref]$r, $size) -eq 0) { return $r }
  if ([WbWin32v2]::GetWindowRect($Handle, [ref]$r)) { return $r }
  return $null
}

function Get-WbWindowBitmap([IntPtr]$Handle) {
  $r = Get-WbWindowBounds $Handle
  if ($null -eq $r) { return $null }
  $w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
  if ($w -le 0 -or $h -le 0) { return $null }
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
  $g.Dispose()
  $bmp
}

# Rendered = the charcoal header (--surface-header, #353A3E) covers at least 3% of a sample grid and
# the window shows at least 12 distinct colours (a blank WebView shows 1 to 3).
function Test-WbRendered([System.Drawing.Bitmap]$Bmp) {
  $charcoal = 0; $colours = @{}; $n = 0
  for ($y = 0; $y -lt 40; $y++) {
    for ($x = 0; $x -lt 60; $x++) {
      $px = $Bmp.GetPixel([int](($x + 0.5) * $Bmp.Width / 60), [int](($y + 0.5) * $Bmp.Height / 40))
      $n++
      $colours["$($px.R),$($px.G),$($px.B)"] = $true
      if ([math]::Abs($px.R - 0x35) -le 6 -and [math]::Abs($px.G - 0x3A) -le 6 -and [math]::Abs($px.B - 0x3E) -le 6) { $charcoal++ }
    }
  }
  [pscustomobject]@{ Rendered = ($charcoal / $n -ge 0.03 -and $colours.Count -ge 12); Charcoal = [math]::Round(100 * $charcoal / $n, 1); Colours = $colours.Count }
}

# Screenshot the window and test it; optionally save the PNG. The capture copies the screen, so it
# runs only when the app's window is in front: otherwise (a dialog, notification or another app on
# top, for example while someone uses the machine) nothing is captured or saved and the result says
# Obscured, so no other window's content can end up in committed evidence.
function Get-WbWindowCheck([IntPtr]$Handle, [string]$SavePath) {
  $front = $false
  for ($try = 0; $try -lt 3 -and -not $front; $try++) {
    [void][WbWin32v2]::SetForegroundWindow($Handle); Start-Sleep -Milliseconds 300
    $front = ([WbWin32v2]::GetForegroundWindow() -eq $Handle)
  }
  if (-not $front) { return [pscustomobject]@{ Rendered = $false; Charcoal = $null; Colours = $null; Obscured = $true } }
  $bmp = Get-WbWindowBitmap $Handle
  if (-not $bmp) { return $null }
  $check = Test-WbRendered $bmp
  if ($SavePath) { $bmp.Save($SavePath, [System.Drawing.Imaging.ImageFormat]::Png) }
  $bmp.Dispose()
  $check | Add-Member -NotePropertyName Obscured -NotePropertyValue $false -PassThru
}

# Start the exe and wait for the window and the first render.
function Start-WbApp {
  param([Parameter(Mandatory)] [string]$Exe, [int]$TimeoutSeconds = 20)
  $watch = [System.Diagnostics.Stopwatch]::StartNew()
  $launchedAt = Get-Date
  $app = Start-Process -FilePath $Exe -PassThru
  $windowMs = $null; $renderMs = $null; $handle = [IntPtr]::Zero
  while ($watch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
    if ($app.HasExited) { break }
    $app.Refresh()
    if ($null -eq $windowMs -and $app.MainWindowHandle -ne [IntPtr]::Zero -and [WbWin32v2]::IsWindowVisible($app.MainWindowHandle)) {
      $windowMs = $watch.ElapsedMilliseconds; $handle = $app.MainWindowHandle
      [void][WbWin32v2]::SetForegroundWindow($handle)
    }
    if ($handle -ne [IntPtr]::Zero) {
      $bmp = Get-WbWindowBitmap $handle
      $check = $null
      if ($bmp) { $check = Test-WbRendered $bmp; $bmp.Dispose() }
      if ($check -and $check.Rendered) { $renderMs = $watch.ElapsedMilliseconds; break }
    }
    Start-Sleep -Milliseconds 100
  }
  [pscustomobject]@{ Process = $app; Handle = $handle; WindowMs = $windowMs; RenderMs = $renderMs; LaunchedAt = $launchedAt; Watch = $watch }
}

Export-ModuleMember -Function Get-WbUserDataDir, Get-WbProcessTree, Get-WbWebView2, Stop-WbAppImmediate, Stop-WbApp, `
  Get-WbWindowBounds, Get-WbWindowBitmap, Test-WbRendered, Get-WbWindowCheck, Start-WbApp
