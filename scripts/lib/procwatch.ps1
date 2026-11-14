# Watches for engine processes (wb-engine*.exe) while the app runs its system check, and writes one
# line of JSON per process: when it started and ended, its variant and the request file it was given
# (copied at first sight, because the app deletes it when the cell ends). The system check itself does
# not save per-cell times; this is how the pack gets them without changing the app.
# Polls every 250 ms. Windows PowerShell 5.1, ASCII only.
param(
  [Parameter(Mandatory = $true)] [string]$OutJsonl,
  [Parameter(Mandatory = $true)] [string]$StopFile,
  [Parameter(Mandatory = $true)] [string]$CopyDir,
  [string]$NamePrefix = 'wb-engine'
)
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force -Path $CopyDir | Out-Null
$seen = @{}
$stamp = { (Get-Date).ToUniversalTime().ToString('o') }
function Write-Line($o) { try { Add-Content -LiteralPath $OutJsonl -Value ($o | ConvertTo-Json -Compress -Depth 6) -Encoding UTF8 } catch { } }
while ($true) {
  $stopping = Test-Path -LiteralPath $StopFile
  $procs = @()
  try { $procs = @(Get-CimInstance Win32_Process -Filter "Name LIKE '$NamePrefix%'" -ErrorAction Stop) } catch { }
  $alive = @{}
  foreach ($p in $procs) {
    $id = [int]$p.ProcessId
    $alive[$id] = $true
    if (-not $seen.ContainsKey($id)) {
      $cmd = [string]$p.CommandLine
      $req = ''
      $copy = ''
      if ($cmd -match '--request\s+"?([^"]+?)"?(\s|$)') { $req = $Matches[1] }
      $text = ''
      if ($req) {
        try {
          $copy = Join-Path $CopyDir ("request-{0}.json" -f $id)
          Copy-Item -LiteralPath $req -Destination $copy -ErrorAction Stop
          $text = [System.IO.File]::ReadAllText($copy)
        } catch { $copy = ''; $text = '' }
      }
      $sub = ''
      if ($cmd -match '\s(bench|batch|live|info)(\s|$)') { $sub = $Matches[1] }
      $seen[$id] = [ordered]@{ pid = $id; exe = [string]$p.Name; command = $sub; started = (& $stamp); ended = $null; request_copy = $copy; request = $text }
    }
  }
  foreach ($id in @($seen.Keys)) {
    if (-not $alive.ContainsKey($id) -and $null -eq $seen[$id]['ended']) {
      $seen[$id]['ended'] = (& $stamp)
      Write-Line $seen[$id]
    }
  }
  if ($stopping -and $alive.Count -eq 0) { break }
  Start-Sleep -Milliseconds 250
}
