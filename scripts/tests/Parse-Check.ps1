# Parse-checks PowerShell files with the Windows PowerShell 5.1 parser and rejects non-ASCII bytes.
# Usage: powershell -File scripts\tests\Parse-Check.ps1 -Path <file or folder> [...]
param([Parameter(Mandatory)][string[]]$Path)
$ErrorActionPreference = 'Stop'
$files = @()
# A comma list given with -File arrives as one string.
$Path = @($Path | ForEach-Object { $_ -split ',' } | Where-Object { $_.Trim() } | ForEach-Object { $_.Trim() })
foreach ($p in $Path) {
  if (Test-Path -LiteralPath $p -PathType Container) {
    $files += Get-ChildItem -LiteralPath $p -Recurse -Include *.ps1, *.psm1 | Select-Object -ExpandProperty FullName
  } else {
    $files += (Resolve-Path -LiteralPath $p).Path
  }
}
$bad = 0
foreach ($f in $files) {
  $tokens = $null; $errs = $null
  [void][System.Management.Automation.Language.Parser]::ParseFile($f, [ref]$tokens, [ref]$errs)
  foreach ($e in $errs) { Write-Host ("PARSE {0}:{1} {2}" -f $f, $e.Extent.StartLineNumber, $e.Message); $bad++ }
  $bytes = [System.IO.File]::ReadAllBytes($f)
  $line = 1
  foreach ($b in $bytes) {
    if ($b -eq 10) { $line++ }
    if ($b -gt 127) { Write-Host ("NONASCII {0}:{1}" -f $f, $line); $bad++; break }
  }
}
Write-Host ("checked {0} file(s), {1} problem(s)" -f $files.Count, $bad)
if ($bad -gt 0) { exit 1 }
