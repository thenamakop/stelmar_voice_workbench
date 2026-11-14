<#
  Hashing.psm1: what the hashing gates (R5: the app hashes the model files in the background while it
  records) need beyond the recorder gate: reading the app's own model_hash lines, deciding whether a run
  was really cold, a raw read of the disk, and (administrator only) emptying Windows' standby file cache.

  A purged run counts as cold only if the app read the models at no more than 0.6 times the rate it read
  them at when the cache was warm on the same machine. A rate above that proves the cache was warm; a rate
  below it is consistent with cold but is not proof (SHA-256 itself can be slower than the disk), and the
  evidence says so.

  Windows PowerShell 5.1, ASCII only.
#>

Set-StrictMode -Version 2

# Pure. The model_hash lines of the app log to rows.
function ConvertFrom-PackHashLines {
  [CmdletBinding()]
  param([string[]]$Lines)
  $rows = New-Object System.Collections.Generic.List[object]
  foreach ($l in @($Lines)) {
    if ($l -match 'model_hash file=(\S+) bytes=(\d+) start_ms=(\d+) end_ms=(\d+) took_ms=(\d+) mb_per_s=(\d+) result=(\w+) background=(\w+)') {
      $rows.Add([pscustomobject]@{ File = $Matches[1]; Bytes = [int64]$Matches[2]; StartMs = [int64]$Matches[3]; EndMs = [int64]$Matches[4]; TookMs = [int64]$Matches[5]; MbPerS = [int]$Matches[6]; Result = $Matches[7]; Background = ($Matches[8] -eq 'true') })
    }
  }
  @($rows.ToArray())
}

# Pure. The table the hashing gate writes into its evidence ("  file | bytes | start | took ms | MB/s |
# result | background | memory priority") to the same rows.
function ConvertFrom-PackHashEvidence {
  [CmdletBinding()]
  param([string[]]$Lines)
  $rows = New-Object System.Collections.Generic.List[object]
  foreach ($l in @($Lines)) {
    if ($l -match '^\s+(\S+\.\w+) \| (\d+) \| ([\d,\.\-]+) \| (\d+) \| (\d+) \| (\w+) \| (\w+) \| (\S+)\s*$') {
      $rows.Add([pscustomobject]@{ File = $Matches[1]; Bytes = [int64]$Matches[2]; StartMs = 0L; EndMs = 0L; TookMs = [int64]$Matches[4]; MbPerS = [int]$Matches[5]; Result = $Matches[6]; Background = ($Matches[7] -eq 'True') })
    }
  }
  @($rows.ToArray())
}

# Pure. Bytes over seconds of the files that hashed ok: MB/s (10^6 bytes), or $null.
function Get-PackHashRate {
  [CmdletBinding()]
  param($Rows)
  $ok = @(@($Rows) | Where-Object { $_.Result -eq 'ok' -and $_.TookMs -gt 0 })
  if ($ok.Count -eq 0) { return $null }
  $bytes = ($ok | Measure-Object -Property Bytes -Sum).Sum
  $ms = ($ok | Measure-Object -Property TookMs -Sum).Sum
  [double]$bytes / 1e6 / ([double]$ms / 1000.0)
}

# Pure. Does a purged run count as cold? -WarmMBs is the same machine's rate with a warm cache.
function Test-PackColdRun {
  [CmdletBinding()]
  param($ColdMBs, $WarmMBs, [double]$Factor = 0.6)
  if ($null -eq $ColdMBs) { return [pscustomobject]@{ Counts = $false; Ratio = $null; Reason = 'no model_hash line was read from the app log, so the rate is unknown' } }
  if ($null -eq $WarmMBs -or [double]$WarmMBs -le 0) { return [pscustomobject]@{ Counts = $false; Ratio = $null; Reason = 'no warm-cache rate was measured on this machine, so the run cannot be called cold' } }
  $ratio = [double]$ColdMBs / [double]$WarmMBs
  if ($ratio -le $Factor) {
    return [pscustomobject]@{ Counts = $true; Ratio = $ratio; Reason = ('the app read the models at {0:n0} MB/s, {1:n2} times the warm rate of {2:n0} MB/s (at most {3:n1}): consistent with a cold cache; this is not proof, because SHA-256 can itself be slower than the disk' -f $ColdMBs, $ratio, $WarmMBs, $Factor) }
  }
  [pscustomobject]@{ Counts = $false; Ratio = $ratio; Reason = ('the app read the models at {0:n0} MB/s, {1:n2} times the warm rate of {2:n0} MB/s (limit {3:n1}): the file cache was warm, so this run does not count as cold' -f $ColdMBs, $ratio, $WarmMBs, $Factor) }
}

# Reads -Bytes from the end of a file in 1 MB blocks and returns MB/s. Used on a file the app is about to
# hash, so it is the disk's own speed when the cache is cold (after a restart or a purge). Logged only.
function Measure-PackReadRate {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Path, [int64]$Bytes = 128MB)
  $fs = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
  try {
    $n = [math]::Min($Bytes, $fs.Length)
    [void]$fs.Seek($fs.Length - $n, 'Begin')
    $buf = New-Object byte[] (1MB)
    $got = [int64]0
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($got -lt $n) { $r = $fs.Read($buf, 0, $buf.Length); if ($r -le 0) { break }; $got += $r }
    $sw.Stop()
    if ($sw.Elapsed.TotalSeconds -le 0) { return $null }
    [double]$got / 1e6 / $sw.Elapsed.TotalSeconds
  } finally { $fs.Dispose() }
}

# Streams a file through SHA-256 in the foreground and returns MB/s: the warm-cache reference for a model
# that was read once before. Needs no administrator rights.
function Measure-PackWarmHashRate {
  [CmdletBinding()]
  param([Parameter(Mandatory = $true)] [string]$Path, [int64]$Bytes = 512MB)
  $sha = [System.Security.Cryptography.SHA256]::Create()
  try {
    $rate = $null
    foreach ($pass in 1, 2) {
      $fs = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
      try {
        $n = [math]::Min($Bytes, $fs.Length)
        $buf = New-Object byte[] (4MB)
        $got = [int64]0
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while ($got -lt $n) { $r = $fs.Read($buf, 0, [int][math]::Min($buf.Length, $n - $got)); if ($r -le 0) { break }; [void]$sha.TransformBlock($buf, 0, $r, $null, 0); $got += $r }
        $sw.Stop()
        if ($pass -eq 2 -and $sw.Elapsed.TotalSeconds -gt 0) { $rate = [double]$got / 1e6 / $sw.Elapsed.TotalSeconds }
      } finally { $fs.Dispose() }
    }
    $rate
  } finally { $sha.Dispose() }
}

if (-not ('PackStandby' -as [type])) {
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PackStandby {
  [DllImport("ntdll.dll")] static extern int NtSetSystemInformation(int cls, IntPtr info, int len);
  [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr h, uint access, out IntPtr tok);
  [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool LookupPrivilegeValue(string sys, string name, out long luid);
  [DllImport("advapi32.dll", SetLastError = true)] static extern bool AdjustTokenPrivileges(IntPtr tok, bool disableAll, ref TokPriv newState, int len, IntPtr prev, IntPtr ret);
  [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [StructLayout(LayoutKind.Sequential, Pack = 1)] struct TokPriv { public int Count; public long Luid; public int Attr; }
  // Returns 0 on success, otherwise an NTSTATUS or -1 when the privilege could not be enabled.
  public static int Purge() {
    IntPtr tok;
    if (!OpenProcessToken(GetCurrentProcess(), 0x28, out tok)) return -1;
    try {
      long luid;
      if (!LookupPrivilegeValue(null, "SeProfileSingleProcessPrivilege", out luid)) return -1;
      TokPriv tp = new TokPriv(); tp.Count = 1; tp.Luid = luid; tp.Attr = 2;
      if (!AdjustTokenPrivileges(tok, false, ref tp, 0, IntPtr.Zero, IntPtr.Zero)) return -1;
      IntPtr p = Marshal.AllocHGlobal(4);
      try { Marshal.WriteInt32(p, 4); return NtSetSystemInformation(80, p, 4); } finally { Marshal.FreeHGlobal(p); }
    } finally { CloseHandle(tok); }
  }
}
'@
}

# Empties Windows' standby list (the file cache of files nobody has open). Administrator only. Returns
# $true when Windows accepted it.
function Clear-PackStandbyList {
  [CmdletBinding()]
  param()
  try { return ([PackStandby]::Purge() -eq 0) } catch { return $false }
}

Export-ModuleMember -Function ConvertFrom-PackHashEvidence, ConvertFrom-PackHashLines, Get-PackHashRate, Test-PackColdRun, Measure-PackReadRate, Measure-PackWarmHashRate, Clear-PackStandbyList
