<#
  ProcWatch.psm1: per-cell times of the system check, taken from outside the app.

  Start-PackProcWatch starts scripts/lib/procwatch.ps1, which records every wb-engine process (start,
  end, request file). Get-PackBenchCells turns those lines into one row per bench cell: model, language,
  device, threads, mode, clip and the seconds the engine process lived. The seconds include the model
  load and the warm-up run, which is what the 480-second bar of the quick check counts.

  Windows PowerShell 5.1, ASCII only.
#>

# Pure. The first property called $Name anywhere inside a parsed JSON object, or $null.
function Find-PackProp {
  param($Object, [string]$Name, [int]$Depth = 0)
  if ($null -eq $Object -or $Depth -gt 6) { return $null }
  if ($Object -is [string] -or $Object -is [ValueType]) { return $null }
  if ($Object -is [System.Collections.IEnumerable] -and $Object -isnot [System.Collections.IDictionary]) {
    foreach ($i in $Object) { $r = Find-PackProp -Object $i -Name $Name -Depth ($Depth + 1); if ($null -ne $r) { return $r } }
    return $null
  }
  $p = $Object.PSObject.Properties[$Name]
  if ($null -ne $p) { return $p.Value }
  foreach ($q in $Object.PSObject.Properties) { $r = Find-PackProp -Object $q.Value -Name $Name -Depth ($Depth + 1); if ($null -ne $r) { return $r } }
  $null
}

function ConvertTo-PackShort {
  param($V)
  if ($null -eq $V) { return '' }
  if ($V -is [string] -or $V -is [ValueType]) { return [string]$V }
  foreach ($n in @('id', 'name', 'file', 'path', 'kind')) {
    $p = $V.PSObject.Properties[$n]
    if ($null -ne $p -and $p.Value) { return [string]$p.Value }
  }
  ($V | ConvertTo-Json -Compress -Depth 3)
}

# Pure. One row per recorded process line. -Lines are the JSONL lines of the watcher.
function Get-PackBenchCells {
  [CmdletBinding()]
  param([string[]]$Lines)
  $rows = New-Object System.Collections.Generic.List[object]
  foreach ($l in @($Lines)) {
    if (-not $l -or -not $l.Trim().StartsWith('{')) { continue }
    $o = $null
    try { $o = $l | ConvertFrom-Json } catch { continue }
    $started = [datetime]::Parse([string]$o.started, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind)
    $ended = $null
    if ($o.ended) { $ended = [datetime]::Parse([string]$o.ended, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind) }
    $req = $null
    if ($o.request) { try { $req = [string]$o.request | ConvertFrom-Json } catch { $req = $null } }
    $dev = Find-PackProp -Object $req -Name 'device'
    $rows.Add([pscustomobject]@{
        Pid = [int]$o.pid; Exe = [string]$o.exe; Command = [string]$o.command
        Started = $started; Seconds = $(if ($ended) { [math]::Round(($ended - $started).TotalSeconds, 2) } else { $null })
        Model = (ConvertTo-PackShort (Find-PackProp -Object $req -Name 'model'))
        Lang = (ConvertTo-PackShort (Find-PackProp -Object $req -Name 'lang'))
        Device = (ConvertTo-PackShort $dev)
        Threads = (Find-PackProp -Object $req -Name 'threads')
        Mode = (ConvertTo-PackShort (Find-PackProp -Object $req -Name 'mode'))
        Clip = $(
          $clipText = ConvertTo-PackShort (Find-PackProp -Object $req -Name 'clip')
          if ($clipText) { Split-Path -Leaf $clipText } else { '' })
      })
  }
  @($rows.ToArray() | Sort-Object Started)
}

# Pure. Total seconds of the bench cells, the longest one, and how much of the check they cover.
function Get-PackCellSummary {
  param($Cells, [double]$CheckSeconds = 0)
  $b = @($Cells | Where-Object { $_.Command -eq 'bench' -and $null -ne $_.Seconds })
  $sum = ($b | Measure-Object -Property Seconds -Sum).Sum
  if ($null -eq $sum) { $sum = 0 }
  $max = ($b | Measure-Object -Property Seconds -Maximum).Maximum
  $longest = $(if ($max) { @($b | Where-Object { $_.Seconds -eq $max })[0] } else { $null })
  [pscustomobject]@{
    Cells = $b.Count; TotalSeconds = [double]$sum
    LongestSeconds = $(if ($max) { [double]$max } else { 0.0 })
    Longest = $(if ($longest) { '{0} {1} on {2}' -f $longest.Model, $longest.Lang, $longest.Device } else { '' })
    CoverPct = $(if ($CheckSeconds -gt 0) { 100.0 * $sum / $CheckSeconds } else { $null })
  }
}

function Start-PackProcWatch {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Dir)
  New-Item -ItemType Directory -Force -Path $Dir | Out-Null
  $out = Join-Path $Dir 'cells.jsonl'
  $stop = Join-Path $Dir 'STOP-WATCH'
  Remove-Item -LiteralPath $out, $stop -Force -ErrorAction SilentlyContinue
  $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $p = Start-Process -FilePath $ps -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $PSScriptRoot 'procwatch.ps1')), '-OutJsonl', ('"{0}"' -f $out), '-StopFile', ('"{0}"' -f $stop), '-CopyDir', ('"{0}"' -f (Join-Path $Dir 'requests')))
  $null = $p.Handle
  [pscustomobject]@{ Process = $p; Out = $out; Stop = $stop }
}

function Stop-PackProcWatch {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] $Handle)
  Set-Content -LiteralPath $Handle.Stop -Value 'stop' -Encoding ascii
  if (-not $Handle.Process.WaitForExit(20000)) { try { $Handle.Process.Kill() } catch { } }
  $lines = @()
  if (Test-Path -LiteralPath $Handle.Out) { $lines = @(Get-Content -LiteralPath $Handle.Out -Encoding UTF8) }
  @(Get-PackBenchCells -Lines $lines)
}

Export-ModuleMember -Function Find-PackProp, Get-PackBenchCells, Get-PackCellSummary, Start-PackProcWatch, Stop-PackProcWatch
