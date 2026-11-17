<#
.SYNOPSIS
  Reads the results zip of a gate-pack run and updates docs\GATES.md and docs\evidence\.

.DESCRIPTION
  1. Checks the zip (every file's SHA-256 against ZIP-MANIFEST.json) and reads summary.json.
  2. Copies the run's logs, reports and capability files to docs\evidence\gate-pack\<machine>\ (a dry run
     on the Legion goes to docs\evidence\gate-pack-dryrun\<profile>\), removing user paths and secret
     values on the way. Evidence files are written by this script only. -Redact names more strings to remove. -AsDryRun <label> files a native
     run on the Legion (the microphone test) the same way, under that label; on an emulated zip it only
     gives the evidence its own folder (a re-run of a few gates).
  3. Real laptop run: the rows of docs\GATES.md named by each gate's ledger entry change from PROVISIONAL
     or DEFERRED to PASS or FAIL only when every laptop and every gate the row needs has a verdict; the
     old status stays in the cell as "[was: ...]". A FAIL adds a "Phase N REOPENED" row to the Open
     items. INFO results never close anything. A run on the VB-CABLE fallback rig leaves the rows that
     need the hardware audio interface open. The Machines table is filled for the laptop.
  4. Every run, real or emulated, also gets one row per gate in the "Gate pack runs" block. An emulated
     run changes no other row.
  -Preview prints what would change and writes nothing.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\ingest-gate-results.ps1 -Zip D:\usb\gate-results-X.zip -Preview
  powershell -ExecutionPolicy Bypass -File .\scripts\ingest-gate-results.ps1 -Zip D:\usb\gate-results-X.zip
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$Zip,
  [ValidateSet('', 'low', 'mid')] [string]$Role = '',
  [switch]$Preview,
  # A run on the Legion that is not an emulation (the microphone test on the Legion's own microphone runs
  # natively): recorded as a dry run under this label, never as a laptop. Rows 84 and 111 and the Machines
  # table are not touched. Example: -AsDryRun legion-native-mic
  [string]$AsDryRun = '',
  # Strings to remove from everything kept in the repository, besides what the redaction always removes: the laptop's
  # computer name and user name when you know them (an identity.json at the root of the zip, if a pack has one, is read too).
  [string[]]$Redact = @(),
  [string]$Repo = '',
  [string]$GatesMd = '',
  [string]$EvidenceRoot = ''
)
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Repo) { $Repo = (Resolve-Path (Join-Path $here '..')).Path }
if (-not $GatesMd) { $GatesMd = Join-Path $Repo 'docs\GATES.md' }
if (-not $EvidenceRoot) { $EvidenceRoot = Join-Path $Repo 'docs\evidence' }
Import-Module (Join-Path $Repo 'scripts\lib\Ingest.psm1') -Force
Import-Module (Join-Path $Repo 'scripts\lib\PackCore.psm1') -Force
Import-Module (Join-Path $Repo 'scripts\lib\PackNative.psm1') -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Fail([string]$m) { Write-Host "ingest-gate-results: $m" -ForegroundColor Red; exit 1 }
if (-not (Test-Path -LiteralPath $Zip)) { Fail "no such file: $Zip" }

# ---- 1. Unpack and verify ----
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('wb-ingest-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  [System.IO.Compression.ZipFile]::ExtractToDirectory((Resolve-Path $Zip).Path, $tmp)
  $zm = Read-PackJson -Path (Join-Path $tmp 'ZIP-MANIFEST.json')
  if ($null -eq $zm) { Fail 'the zip has no ZIP-MANIFEST.json: it was not written by the gate pack' }
  $bad = New-Object System.Collections.Generic.List[string]
  foreach ($f in @($zm.files)) {
    $p = Join-Path $tmp (([string]$f.path) -replace '/', '\')
    if (-not (Test-Path -LiteralPath $p)) { $bad.Add("missing: $($f.path)"); continue }
    if ((Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant() -ne [string]$f.sha256) { $bad.Add("changed: $($f.path)") }
  }
  if ($bad.Count -gt 0) { Fail ("the zip does not match its ZIP-MANIFEST.json:`n  " + ($bad -join "`n  ")) }
  $summary = Read-PackJson -Path (Join-Path $tmp 'summary.json')
  if ($null -eq $summary) { Fail 'the zip has no summary.json' }
  # What to remove besides the always-removed kinds: -Redact; the computer name and user name the zip records, if it does
  # (identity.json at the zip root, or computer_name / hostname in summary.json or its fingerprint); never copied.
  $extra = New-Object System.Collections.Generic.List[string]
  $hostNames = New-Object System.Collections.Generic.List[string]
  $userNames = New-Object System.Collections.Generic.List[string]
  foreach ($x in @($Redact | ForEach-Object { $_ -split ',' })) { if ($x.Trim()) { $extra.Add($x.Trim()) } }
  $idFile = Join-Path $tmp 'identity.json'
  foreach ($src in @($(if (Test-Path -LiteralPath $idFile) { Read-PackJson -Path $idFile }), $summary, $summary.fingerprint)) {
    if ($null -eq $src) { continue }
    foreach ($k in 'computername', 'computer_name', 'hostname', 'host_name') { if ($src.PSObject.Properties[$k] -and [string]$src.$k) { $hostNames.Add([string]$src.$k) } }
    foreach ($k in 'username', 'user_name', 'userdomain') { if ($src.PSObject.Properties[$k] -and [string]$src.$k) { $userNames.Add([string]$src.$k) } }
  }
  $extraArr = @($extra.ToArray())
  $redactArgs = @{ Extra = $extraArr; ComputerName = @($hostNames.ToArray()); UserName = @($userNames.ToArray()) }
  if ([int]$summary.pack_version -ne 1) { Fail "pack_version $($summary.pack_version) is not understood by this script" }
  # Windows PowerShell turns ISO date strings into dates; one stable text keeps the run id the same.
  $summary.created = ([datetime]$summary.created).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ', [Globalization.CultureInfo]::InvariantCulture)
  if (-not $Role) { $Role = [string]$summary.role }
  if ($Role -notin 'low', 'mid') { Fail 'the zip does not say which laptop it came from; pass -Role low or -Role mid' }
  $emu = [string]$summary.emulated
  $evName = $emu
  if ($AsDryRun) {
    if ($AsDryRun -notmatch '^[A-Za-z0-9._-]+$') { Fail '-AsDryRun takes a short label of letters, digits, dot, dash and underscore' }
    # A native run on the Legion becomes a dry run under the label; an emulated run keeps its profile and only gets
    # its own evidence folder, so a re-run of two gates does not overwrite the files of the earlier run.
    if (-not $emu) { $emu = $AsDryRun; $summary.emulated = $AsDryRun }
    $evName = $AsDryRun
  }
  $created = [datetime]::Parse([string]$summary.created, [Globalization.CultureInfo]::InvariantCulture).ToLocalTime()
  $date = $created.ToString('d MMM yyyy', [Globalization.CultureInfo]::InvariantCulture)
  $machine = ([string]$summary.machine -replace '[^A-Za-z0-9._-]', '-')
  $evRel = $(if ($emu) { "docs/evidence/gate-pack-dryrun/$evName" } else { "docs/evidence/gate-pack/$machine" })
  $evDir = Join-Path $Repo ($evRel -replace '/', '\')
  Write-Host ("zip: {0}" -f (Split-Path -Leaf $Zip))
  Write-Host ("run: {0} laptop, machine {1}{2}; pack commit {3}; created {4}; {5}" -f $Role, $summary.machine, $(if ($emu) { " [EMULATED: $emu on the Legion]" } else { '' }), $summary.source_commit, $date, $(if ($summary.quick) { '-Quick (not at full length)' } else { 'full length' }))
  $counts = @($summary.gates | Group-Object status | ForEach-Object { "$($_.Count) $($_.Name)" }) -join ', '
  Write-Host ("gates: {0}; waiting {1}; not in this pack {2}" -f $counts, @($summary.pending).Count, @($summary.not_in_pack).Count)

  # ---- 2b. The Python reporters (system_check_report.py and the speed-table recomputation) ----
  # They read the raw reports of the zip and write evidence files; a consistency problem lowers a PASS to
  # FAIL or an INFO to INVALID before the rows are written. They never raise a status.
  $py = Find-IngestPython -Repo $Repo
  $repOut = Join-Path $tmp 'reports-out'
  if (-not $py) {
    Write-Host 'reporters not run: the evaluation environment (eval\.venv) was not found; the statuses are the laptop''s own' -ForegroundColor Yellow
  } else {
    $rp = Invoke-PackNative -FilePath $py -ArgumentList @('-X', 'utf8', (Join-Path $Repo 'scripts\gates\pack_reports.py'), (Join-Path $tmp 'work'), $repOut) -TimeoutSeconds 600
    if ($rp.ExitCode -ne 0) {
      Write-Host ('reporters failed (exit {0}): {1}' -f $rp.ExitCode, ((@($rp.Error) + @($rp.Output)) -join ' ')) -ForegroundColor Yellow
    } else {
      Write-Host ($rp.Output -join ' ')
      $find = Read-PackJson -Path (Join-Path $repOut 'consistency.json')
      foreach ($l in @(Set-IngestReporterFindings -Summary $summary -Findings $find)) { Write-Host $l -ForegroundColor Yellow }
    }
  }

  # ---- 2. Evidence ----
  $textExt = '.txt', '.json', '.log', '.csv', '.md', '.jsonl', '.out', '.err'
  $copied = 0
  if (-not $Preview) {
    New-Item -ItemType Directory -Force -Path $evDir | Out-Null
    foreach ($f in @(Get-ChildItem -LiteralPath (Join-Path $tmp 'work') -Recurse -File -ErrorAction SilentlyContinue)) {
      # Programs, libraries and audio are never evidence (an older zip may carry engine copies).
      # (.renamed is an engine program the rename-engines gate moved aside; on 17 Nov 2026 it reached the evidence folder.)
      if (@('.exe', '.dll', '.pdb', '.sys', '.wav', '.bin', '.renamed', '.dmp', '.zip', '.gguf') -contains $f.Extension.ToLowerInvariant()) { continue }
      if ($f.FullName -match '\\scratch\\' -and $textExt -notcontains $f.Extension.ToLowerInvariant()) { continue }
      $rel = $f.FullName.Substring((Join-Path $tmp 'work').Length).TrimStart('\')
      $to = Join-Path $evDir $rel
      New-Item -ItemType Directory -Force -Path (Split-Path -Parent $to) | Out-Null
      if ($textExt -contains $f.Extension.ToLowerInvariant()) {
        $lines = @(Get-Content -LiteralPath $f.FullName -Encoding UTF8)
        [IO.File]::WriteAllText($to, ((@(ConvertTo-IngestRedacted -Line $lines @redactArgs) -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding($false)))
      } else { Copy-Item -LiteralPath $f.FullName -Destination $to -Force }
      $copied++
    }
    [IO.File]::WriteAllText((Join-Path $evDir 'summary.json'), ((@(ConvertTo-IngestRedacted -Line @(Get-Content -LiteralPath (Join-Path $tmp 'summary.json') -Encoding UTF8) @redactArgs) -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding($false)))
    if (Test-Path -LiteralPath $repOut) {
      New-Item -ItemType Directory -Force -Path (Join-Path $evDir 'reports') | Out-Null
      foreach ($f in @(Get-ChildItem -LiteralPath $repOut -File)) { [IO.File]::WriteAllText((Join-Path (Join-Path $evDir 'reports') $f.Name), ((@(ConvertTo-IngestRedacted -Line @(Get-Content -LiteralPath $f.FullName -Encoding UTF8) @redactArgs) -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding($false))) }
    }
    # A short reading of the summary, written by this script.
    $txt = New-Object System.Collections.Generic.List[string]
    $txt.Add("Gate pack run: $Role laptop, machine $($summary.machine), created $($summary.created)")
    $txt.Add("Pack commit $($summary.source_commit); $(if ($emu) { "EMULATED ($emu), not a laptop" } else { 'real laptop' }); $(if ($summary.quick) { '-Quick, not at full length' } else { 'full length' })")
    foreach ($g in @($summary.gates)) { $txt.Add(('{0,-8} {1,-20} {2}' -f $g.status, $g.id, $g.headline)) }
    foreach ($p in @($summary.pending)) { $txt.Add(('WAITING  {0,-20} {1}' -f $p.id, $p.reason)) }
    foreach ($p in @($summary.not_in_pack)) { $txt.Add(('NOT IN THIS PACK  {0}' -f $p.id)) }
    if (@($summary.still_changed).Count -gt 0) { $txt.Add('LEFT CHANGED ON THE LAPTOP:'); foreach ($s in @($summary.still_changed)) { $txt.Add("  $s") } }
    [IO.File]::WriteAllText((Join-Path $evDir 'RESULTS.txt'), ((@(ConvertTo-IngestRedacted -Line @($txt.ToArray()) @redactArgs) -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding($false)))
    Write-Host "evidence: $copied files to $evRel"
  } else { Write-Host "preview: evidence would go to $evRel" }

  # The words of the ledger rows come from the summary: the same removal.
  foreach ($g in @($summary.gates)) { foreach ($k in 'headline', 'rig') { if ($g.PSObject.Properties[$k] -and $g.$k) { $g.$k = [string](@(ConvertTo-IngestRedacted -Line @([string]$g.$k) @redactArgs)[0]) } } }
  foreach ($p in @($summary.pending)) { if ($p.PSObject.Properties['reason'] -and $p.reason) { $p.reason = [string](@(ConvertTo-IngestRedacted -Line @([string]$p.reason) @redactArgs)[0]) } }

  # ---- 3 and 4. docs\GATES.md ----
  $raw = [IO.File]::ReadAllText($GatesMd)
  $eol = $(if ($raw.Contains("`r`n")) { "`r`n" } else { "`n" })
  $trail = $raw.EndsWith($eol)
  $lines = @($raw -split "`r?`n")
  if ($trail -and $lines.Count -gt 0 -and $lines[-1] -eq '') { $lines = @($lines[0..($lines.Count - 2)]) }
  $changes = New-Object System.Collections.Generic.List[string]
  $newLines = $lines
  if (-not $emu) {
    $u = Update-IngestGatesLines -Lines $lines -Summary $summary -Role $Role -Date $date -EvidenceDir $evRel
    $newLines = $u.Lines
    foreach ($c in $u.Changes) { $changes.Add($c) }
    $m = Update-IngestMachines -Lines $newLines -Summary $summary -Role $Role -Date $date
    $newLines = $m.Lines
    if ($m.Changed) { $changes.Add("  Machines table: filled in the $Role laptop") }
    if ($u.Reopened.Count -gt 0) {
      Write-Host ''
      Write-Host ('=' * 78) -ForegroundColor Red
      foreach ($r in $u.Reopened) { Write-Host "  GATE FAILED: $r on $($summary.machine): its phase is REOPENED (row added to the Open items)" -ForegroundColor Red }
      Write-Host ('=' * 78) -ForegroundColor Red
    }
  } else { $changes.Add('  an emulated run: the rows of the Gates and Open items tables are not touched') }
  $newLines = Update-IngestRunsBlock -Lines $newLines -Summary $summary -Role $Role -Date $date -EvidenceDir $evRel
  $diffCount = 0
  $max = [math]::Max($lines.Count, $newLines.Count)
  for ($i = 0; $i -lt $max; $i++) { $a = $(if ($i -lt $lines.Count) { $lines[$i] } else { $null }); $b = $(if ($i -lt $newLines.Count) { $newLines[$i] } else { $null }); if ($a -ne $b) { $diffCount++ } }
  Write-Host ''
  Write-Host ("docs\GATES.md: {0} line(s) change{1}" -f $diffCount, $(if ($Preview) { ' (preview, nothing written)' } else { '' }))
  foreach ($c in $changes) { Write-Host $c }
  $gatesTouched = @($newLines | Where-Object { $_.IndexOf("[run $($summary.created)]", [StringComparison]::Ordinal) -ge 0 }).Count
  Write-Host ("  Gate pack runs block: {0} row(s) for this run" -f $gatesTouched)
  if (-not $Preview) {
    [IO.File]::WriteAllText($GatesMd, (($newLines -join $eol) + $(if ($trail) { $eol } else { '' })), (New-Object Text.UTF8Encoding($false)))
    Write-Host 'written.'
  }
  $wait = @($summary.pending | ForEach-Object { $_.id })
  if ($wait.Count -gt 0) { Write-Host ("still waiting (no verdict, nothing closed): " + ($wait -join ', ')) -ForegroundColor Yellow }
  if (@($summary.still_changed).Count -gt 0) { Write-Host 'THE LAPTOP SAYS IT STILL HAS CHANGED SETTINGS (see RESULTS.txt).' -ForegroundColor Yellow }
} finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }
