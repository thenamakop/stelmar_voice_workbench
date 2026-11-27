<#
.SYNOPSIS
  Proves that no engine on disk contains a language-rule experiment switch (Phase 5, items 4 and 5; kept as a guard).

.DESCRIPTION
  In Phase 5 the per-chunk language rule was tried with experiments behind the cargo feature lang-experiments, switched by
  the environment variable WB_LANG_EXPERIMENT (removed on 25 Nov 2026 in the same work that chose the rule). A release
  engine must never carry such a switch: nothing may change how an engine decodes from the environment of a room laptop.
  This script reads every engine in src-tauri\binaries as bytes and looks for the variable name and the old feature's
  strings, as ASCII and as UTF-16. Every engine must contain NONE of them.

  The scan proves itself first: it writes a small file holding each needle in both encodings and fails unless it finds
  every one, so a clean result cannot come from a scanner that is blind. Exit 0 when the self-test holds and every engine
  is clean. The output is evidence: the script writes it, nobody edits it (-Out docs\evidence\phase-5\engine-no-experiments.txt).
#>
[CmdletBinding()]
param(
  [string]$BinDir,
  [string]$Out
)
$ErrorActionPreference = 'Stop'
if (-not $BinDir) {
  $here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
  $BinDir = Join-Path (Split-Path -Parent $here) 'src-tauri\binaries'
}
$needles = @('WB_LANG_EXPERIMENT', 'lang-experiments', 'lang::experiment', 'always_both', 'DETECT_WITH_HINDI_MODEL')

function Find-Strings([string]$Path, [string[]]$Text) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $ascii = [System.Text.Encoding]::GetEncoding(28591).GetString($bytes)
  $found = @()
  foreach ($t in $Text) {
    $wide = -join ($t.ToCharArray() | ForEach-Object { [string]$_ + [char]0 })
    if ($ascii.IndexOf($t, [System.StringComparison]::Ordinal) -ge 0) { $found += "$t (ASCII)" }
    if ($ascii.IndexOf($wide, [System.StringComparison]::Ordinal) -ge 0) { $found += "$t (UTF-16)" }
  }
  return $found
}

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("Language-rule experiment switch against the engines on disk ($BinDir). Written by scripts/check-engine-no-experiments.ps1.")
$lines.Add("Looked for: $($needles -join ', '), each as ASCII and as UTF-16.")
$problems = New-Object System.Collections.Generic.List[string]

# Self-test: a file that holds every needle in both encodings must be found in full.
$probe = Join-Path ([System.IO.Path]::GetTempPath()) ("engine-scan-probe-" + [guid]::NewGuid().ToString('N') + '.bin')
try {
  $ms = New-Object System.IO.MemoryStream
  foreach ($t in $needles) {
    $a = [System.Text.Encoding]::ASCII.GetBytes($t); $ms.Write($a, 0, $a.Length); $ms.WriteByte(0)
    $w = [System.Text.Encoding]::Unicode.GetBytes($t); $ms.Write($w, 0, $w.Length); $ms.WriteByte(0)
  }
  [System.IO.File]::WriteAllBytes($probe, $ms.ToArray())
  $got = @(Find-Strings $probe $needles)
  $want = $needles.Count * 2
  $lines.Add("self-test: the scan finds $($got.Count) of $want needle encodings in a probe file that holds all of them")
  if ($got.Count -ne $want) { $problems.Add("the scan is blind: it found $($got.Count) of $want probe strings") }
} finally {
  Remove-Item -LiteralPath $probe -Force -ErrorAction SilentlyContinue
}
$lines.Add('')

$engines = @(Get-ChildItem -LiteralPath $BinDir -Filter 'wb-engine-*.exe' -File)
foreach ($f in $engines) {
  $hit = @(Find-Strings $f.FullName $needles)
  $sha = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  $lines.Add(("engine  {0}  {1:n1} MB  {2}  {3}" -f $f.Name, ($f.Length / 1MB), $sha.Substring(0, 16), $(if ($hit.Count) { 'FOUND: ' + ($hit -join '; ') } else { 'clean: none of the strings' })))
  if ($hit.Count) { $problems.Add("$($f.Name) contains the experiment switch: $($hit -join '; ')") }
}
if ($engines.Count -eq 0) { $problems.Add('no engine found to check') }
$lines.Add('')
$lines.Add($(if ($problems.Count -eq 0) { 'RESULT: PASS: the scan finds every probe string and no engine contains the switch.' } else { 'RESULT: FAIL: ' + ($problems -join ' | ') }))
$text = ($lines -join "`r`n") + "`r`n"
if ($Out) {
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null
  [System.IO.File]::WriteAllText($Out, $text, (New-Object System.Text.UTF8Encoding($false)))
}
Write-Host $text
if ($problems.Count -gt 0) { exit 1 }
