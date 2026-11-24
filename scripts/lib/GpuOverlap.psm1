<#
.SYNOPSIS
  The NVIDIA-GPU check of the recorder gates, as pure functions: which nvidia-smi processes belong to the
  emulation Job Object, and a report that names them.
.DESCRIPTION
  An emulated profile (emu-low, emu-mid) hides the NVIDIA GPU from the engines; the gate then checks that no
  process of the emulation Job Object is listed by nvidia-smi. On 23 Nov 2026 one soak failed that check with
  "overlap 1" and the evidence only had counts, so the process could not be named. This module parses the
  nvidia-smi process table, joins it with the job's process ids and the processes' command lines, and gives lines
  that name every nvidia-smi process (pid, type, name, command line cut at 200 characters, in the job or not)
  and a short text naming the overlapping ones for the verdict line.

  Command lines go through ConvertTo-WbRedacted (the profile folder becomes %USERPROFILE%, the user name
  %USERNAME%). The evidence file is redacted again when it is written.

  ASCII only; Windows PowerShell 5.1.
#>

Set-StrictMode -Version 2
Import-Module (Join-Path $PSScriptRoot 'Redact.psm1')

$script:CmdMax = 200

# nvidia-smi's process rows: "|    0   N/A  N/A     12345    C+G   C:\Windows\...\dwm.exe     N/A  |". Only rows of a
# graphics or compute process (C, G, C+G) have a pid; the header and the GPU table do not match.
function ConvertFrom-WbNvidiaSmi {
  param([AllowEmptyCollection()] [AllowNull()] [string[]]$Lines)
  $rows = New-Object System.Collections.Generic.List[object]
  foreach ($l in @($Lines)) {
    if ($l -match '^\|\s+\S+\s+\S+\s+\S+\s+(\d+)\s+(C\+G|C|G)\s+(.*?)\s+\S+\s*\|') {
      $rows.Add([pscustomobject]@{ Pid = [int]$Matches[1]; Type = $Matches[2]; Name = $Matches[3] })
    }
  }
  $rows.ToArray()
}

function Get-WbShortCommandLine {
  param([AllowNull()] [AllowEmptyString()] [string]$CommandLine, [string]$UserProfile = $env:USERPROFILE, [string]$UserName = $env:USERNAME)
  if ([string]::IsNullOrEmpty($CommandLine)) { return '(command line not readable)' }
  $c = ($CommandLine -replace '\s+', ' ').Trim()
  $c = @($c | ConvertTo-WbRedacted -UserProfile $UserProfile -UserName $UserName) -join ' '
  if ($c.Length -gt $script:CmdMax) { $c = $c.Substring(0, $script:CmdMax) + '...' }
  $c
}

# "pid 1234 msedgewebview2.exe: <command line>; pid ..." for rows of Get-WbGpuOverlap (the verdict line uses it).
function Get-WbGpuOverlapText {
  param([AllowEmptyCollection()] [AllowNull()] [object[]]$Rows)
  (@($Rows) | ForEach-Object { "pid {0} {1}: {2}" -f $_.Pid, (Split-Path -Leaf ($_.Name -replace '/', '\')), $_.CommandLine }) -join '; '
}

# $NvProcesses: rows from ConvertFrom-WbNvidiaSmi. $JobPids: process ids of the emulation job (the tree under
# wb-emulate). $CommandLines: hashtable pid -> command line (any may be missing).
# Returns Rows (every nvidia-smi process, with InJob and a redacted, cut command line), Overlap (the rows in the
# job), Lines (one report line per row) and Text (names of the overlapping ones, for the verdict; '' if none).
function Get-WbGpuOverlap {
  param(
    [AllowEmptyCollection()] [AllowNull()] [object[]]$NvProcesses,
    [AllowEmptyCollection()] [AllowNull()] [int[]]$JobPids,
    [hashtable]$CommandLines = @{},
    [string]$UserProfile = $env:USERPROFILE,
    [string]$UserName = $env:USERNAME
  )
  $job = @($JobPids)
  $rows = @(@($NvProcesses) | ForEach-Object {
      $cmd = $(if ($CommandLines.ContainsKey([int]$_.Pid)) { [string]$CommandLines[[int]$_.Pid] } else { $null })
      [pscustomobject]@{
        Pid = [int]$_.Pid; Type = [string]$_.Type; Name = [string]$_.Name
        InJob = ($job -contains [int]$_.Pid)
        CommandLine = (Get-WbShortCommandLine -CommandLine $cmd -UserProfile $UserProfile -UserName $UserName)
      }
    })
  $overlap = @($rows | Where-Object { $_.InJob })
  $lines = @($rows | ForEach-Object {
      $leaf = Split-Path -Leaf ($_.Name -replace '/', '\')
      "nvidia-smi process: pid {0} {1} {2}{3}; command line: {4}" -f $_.Pid, $_.Type, $leaf, $(if ($_.InJob) { ' [IN THE EMULATION JOB]' } else { '' }), $_.CommandLine
    })
  $text = Get-WbGpuOverlapText -Rows $overlap
  [pscustomobject]@{ Rows = $rows; Overlap = $overlap; Lines = $lines; Text = $text }
}

Export-ModuleMember -Function ConvertFrom-WbNvidiaSmi, Get-WbShortCommandLine, Get-WbGpuOverlapText, Get-WbGpuOverlap
