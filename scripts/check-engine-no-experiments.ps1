<#
.SYNOPSIS
  Proves that no shipped engine contains the language-rule experiment switch (Phase 5, items 4 and 5).

.DESCRIPTION
  The experiments (cargo feature lang-experiments of wb-engine-core and wb-engine) read the environment variable
  WB_LANG_EXPERIMENT. A release engine must not contain that code at all. This script reads every engine in
  src-tauri\binaries as bytes and looks for the variable name and the feature's strings, as ASCII and as UTF-16:

    * every shipped engine (wb-engine-<variant>-x86_64-pc-windows-msvc.exe) must contain NONE of them;
    * every experiment engine (wb-engine-<variant>-lang-experiments-...) must contain WB_LANG_EXPERIMENT, which shows that
      the scan can find the switch when it is there (a positive control; with no experiment engine on disk the script says
      so and the control is missing, which fails unless -NoControl is given).

  Exit 0 when every shipped engine is clean and the control holds. The output is evidence: the script writes it, nobody
  edits it (-Out docs\evidence\phase-5\engine-no-experiments.txt).
#>
[CmdletBinding()]
param(
  [string]$BinDir,
  [string]$Out,
  [switch]$NoControl
)
$ErrorActionPreference = 'Stop'
if (-not $BinDir) {
  $here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
  $BinDir = Join-Path (Split-Path -Parent $here) 'src-tauri\binaries'
}
$needles = @('WB_LANG_EXPERIMENT', 'lang-experiments', 'lang::experiment', 'always_both')
$required = 'WB_LANG_EXPERIMENT'

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
$lines.Add('')
$problems = New-Object System.Collections.Generic.List[string]
$shipped = @(Get-ChildItem -LiteralPath $BinDir -Filter 'wb-engine-*.exe' -File | Where-Object { $_.Name -notlike '*lang-experiments*' })
$experiment = @(Get-ChildItem -LiteralPath $BinDir -Filter 'wb-engine-*lang-experiments*.exe' -File)
foreach ($f in $shipped) {
  $hit = @(Find-Strings $f.FullName $needles)
  $sha = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  $lines.Add(("shipped    {0}  {1:n1} MB  {2}  {3}" -f $f.Name, ($f.Length / 1MB), $sha.Substring(0, 16), $(if ($hit.Count) { 'FOUND: ' + ($hit -join '; ') } else { 'clean: none of the strings' })))
  if ($hit.Count) { $problems.Add("$($f.Name) contains the experiment switch: $($hit -join '; ')") }
}
if ($shipped.Count -eq 0) { $problems.Add('no shipped engine found to check') }
$controlOk = $false
foreach ($f in $experiment) {
  $hit = @(Find-Strings $f.FullName @($required))
  $sha = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  $lines.Add(("experiment {0}  {1:n1} MB  {2}  {3}" -f $f.Name, ($f.Length / 1MB), $sha.Substring(0, 16), $(if ($hit.Count) { 'control: the scan finds ' + ($hit -join '; ') } else { 'CONTROL FAILED: the scan did not find the switch' })))
  if ($hit.Count) { $controlOk = $true } else { $problems.Add("control failed on $($f.Name)") }
}
if ($experiment.Count -eq 0) {
  $lines.Add('experiment (none on disk: no positive control)')
  if (-not $NoControl) { $problems.Add('no experiment engine on disk, so the scan has no positive control') }
}
$lines.Add('')
$lines.Add($(if ($problems.Count -eq 0) { 'RESULT: PASS: no shipped engine contains the switch' + $(if ($controlOk) { '; the scan finds it in the experiment engine.' } else { '.' }) } else { 'RESULT: FAIL: ' + ($problems -join ' | ') }))
$text = ($lines -join "`r`n") + "`r`n"
if ($Out) {
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null
  [System.IO.File]::WriteAllText($Out, $text, (New-Object System.Text.UTF8Encoding($false)))
}
Write-Host $text
if ($problems.Count -gt 0) { exit 1 }
