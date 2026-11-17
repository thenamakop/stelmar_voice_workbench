<#
  PackCore.psm1: the engine of the laptop gate pack (scripts/gate-pack, built into gate-pack/ by
  scripts/build-gate-pack.ps1).

  What it holds:
    - the registry (gates.manifest.json): reading it, checking it, and the pure rule that picks the
      next gate (priority, restart and uptime rules, what is missing);
    - the state file that lets the runner resume after a restart or a crash;
    - gate results (one JSON file per gate) and the summary and zip the laptop hands back;
    - the things the runner changes on a laptop (logon task, power use) and how -Cleanup undoes them.

  Nothing here needs a development tool. ASCII only; Windows PowerShell 5.1.
#>

Set-StrictMode -Version 2

Import-Module (Join-Path $PSScriptRoot 'PackNative.psm1')

# ---- Small helpers -------------------------------------------------------------------------------

# A property of a parsed JSON object, or a default when it is absent (StrictMode throws otherwise).
function Get-PackProp {
  param($Object, [string]$Name, $Default = $null)
  if ($null -eq $Object) { return $Default }
  if ($Object -is [System.Collections.IDictionary]) {
    if ($Object.Contains($Name)) { return $Object[$Name] }
    return $Default
  }
  $p = $Object.PSObject.Properties[$Name]
  if ($null -eq $p) { return $Default }
  $p.Value
}

# Parsed JSON (PSCustomObject tree) to nested ordered hashtables and lists, so state can be changed.
function ConvertTo-PackHashtable {
  param($Object)
  if ($null -eq $Object) { return $null }
  if ($Object -is [string] -or $Object -is [ValueType]) { return $Object }
  if ($Object -is [System.Collections.IDictionary]) {
    $h = [ordered]@{}
    foreach ($k in $Object.Keys) { $h[$k] = ConvertTo-PackHashtable $Object[$k] }
    return $h
  }
  if ($Object -is [System.Collections.IEnumerable]) {
    $l = New-Object System.Collections.ArrayList
    foreach ($i in $Object) { [void]$l.Add((ConvertTo-PackHashtable $i)) }
    return , $l
  }
  $o = [ordered]@{}
  foreach ($p in $Object.PSObject.Properties) { $o[$p.Name] = ConvertTo-PackHashtable $p.Value }
  $o
}

function Read-PackJson {
  param([Parameter(Mandatory = $true)] [string]$Path, [switch]$AsHashtable)
  if (-not (Test-Path -LiteralPath $Path)) { return $null }
  $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8
  if (-not $text -or -not $text.Trim()) { return $null }
  $o = $text | ConvertFrom-Json
  if ($AsHashtable) { return (ConvertTo-PackHashtable $o) }
  $o
}

# A line read with Get-Content carries note properties (PSPath, PSDrive, PSProvider ...). Windows PowerShell 5.1
# ConvertTo-Json then writes them as objects: it walks the whole drive and provider (minutes, then "Cannot
# convert value to type System.String"). Every string, hashtable and object is rebuilt from plain values before
# it is written, so no caller has to remember to cast.
function ConvertTo-PackPlainValue {
  param($Value)
  if ($null -eq $Value) { return $null }
  if ($Value -is [string]) { return [string]$Value }
  if ($Value -is [System.Collections.IDictionary]) {
    $o = [ordered]@{}
    foreach ($k in @($Value.Keys)) { $o[[string]$k] = (ConvertTo-PackPlainValue $Value[$k]) }
    return $o
  }
  if ($Value -is [System.Management.Automation.PSCustomObject]) {
    $o = [ordered]@{}
    foreach ($p in @($Value.PSObject.Properties)) { $o[[string]$p.Name] = (ConvertTo-PackPlainValue $p.Value) }
    return $o
  }
  if ($Value -is [System.Collections.IEnumerable]) {
    $items = New-Object System.Collections.Generic.List[object]
    foreach ($i in $Value) { $items.Add((ConvertTo-PackPlainValue $i)) }
    return , $items.ToArray()
  }
  $Value
}

# Written to a temporary file and moved, so a crash or a restart never leaves half a file.
function Write-PackJson {
  param([Parameter(Mandatory = $true)] [string]$Path, [Parameter(Mandatory = $true)] $Value, [int]$Depth = 12)
  $dir = Split-Path -Parent $Path
  if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  $tmp = "$Path.tmp"
  $json = (ConvertTo-PackPlainValue $Value) | ConvertTo-Json -Depth $Depth
  [System.IO.File]::WriteAllText($tmp, $json, (New-Object System.Text.UTF8Encoding($false)))
  Move-Item -LiteralPath $tmp -Destination $Path -Force
}

function Write-PackLog {
  param([string]$Message, [string]$LogFile = $script:PackLogFile)
  $line = '{0} {1}' -f (Get-Date -Format 'HH:mm:ss'), $Message
  Write-Host $line
  if ($LogFile) {
    try { Add-Content -LiteralPath $LogFile -Value $line -Encoding UTF8 } catch { }
  }
}
$script:PackLogFile = $null

# Prints a boxed message to the console (and to a log file when one is given). It lives in this module so
# every caller can reach it: a script block run by Invoke-PackGateProcess cannot see functions defined in
# run-gates.ps1.
function Write-PackBanner {
  param([string[]]$Lines, [string]$Colour = 'Yellow', [string]$LogFile = '')
  $w = 78
  Write-Host ''
  Write-Host ('=' * $w) -ForegroundColor $Colour
  foreach ($l in $Lines) { Write-Host $l -ForegroundColor $Colour }
  Write-Host ('=' * $w) -ForegroundColor $Colour
  if ($LogFile) {
    foreach ($l in $Lines) { Add-Content -LiteralPath $LogFile -Value $l -Encoding UTF8 -ErrorAction SilentlyContinue }
  }
}

# The OnTick handler of a gate that may ask the person something: when the gate has written PROMPT.txt
# in its folder, shows it in a banner, asks for Enter (or a short answer) and writes ANSWER.txt. -Read is
# the console read (a test passes its own).
function New-PackPromptHandler {
  param([Parameter(Mandatory = $true)][string]$GateId, [Parameter(Mandatory = $true)][string]$GateDir, [string]$LogFile = '', [scriptblock]$Read = $null)
  $reader = $Read
  if (-not $reader) { $reader = { param($prompt) Read-Host $prompt } }
  {
    param($proc)
    $pf = Join-Path $GateDir 'PROMPT.txt'
    if (Test-Path -LiteralPath $pf) {
      $msg = (Get-Content -LiteralPath $pf -Raw -Encoding UTF8)
      Remove-Item -LiteralPath $pf -Force
      Write-PackBanner -Lines (@("$GateId needs you:") + @($msg -split "`r?`n" | ForEach-Object { '  ' + $_ })) -Colour 'Cyan' -LogFile $LogFile
      $ans = & $reader 'Press Enter when done (or type a short answer)'
      Set-Content -LiteralPath (Join-Path $GateDir 'ANSWER.txt') -Value $(if ($ans) { $ans } else { 'ok' }) -Encoding UTF8
    }
  }.GetNewClosure()
}

function Set-PackLogFile { param([string]$Path) $script:PackLogFile = $Path }

# ---- The registry --------------------------------------------------------------------------------

$script:ValidGroups = @('any', 'interface', 'cold', 'settled', 'network', 'mic', 'long')
$script:ValidPriorities = @('P1', 'P2', 'P3')

function Get-PackManifestProblems {
  param([Parameter(Mandatory = $true)] $Manifest)
  $problems = New-Object System.Collections.Generic.List[string]
  $gates = @(Get-PackProp $Manifest 'gates' @())
  if ($gates.Count -eq 0) { $problems.Add('the manifest lists no gates') }
  $ids = @{}
  foreach ($g in $gates) {
    $id = [string](Get-PackProp $g 'id' '')
    if (-not $id) { $problems.Add('a gate has no id'); continue }
    if ($ids.ContainsKey($id)) { $problems.Add("duplicate gate id '$id'") }
    $ids[$id] = $true
    if ($id -notmatch '^[a-z0-9][a-z0-9-]*$') { $problems.Add("gate id '$id' must be lower case letters, digits and dashes") }
    $pri = [string](Get-PackProp $g 'priority' '')
    if ($script:ValidPriorities -notcontains $pri) { $problems.Add("gate '$id': priority '$pri' is not P1, P2 or P3") }
    $grp = [string](Get-PackProp $g 'group' '')
    if ($script:ValidGroups -notcontains $grp) { $problems.Add("gate '$id': group '$grp' is not one of $($script:ValidGroups -join ', ')") }
    if ($null -eq (Get-PackProp $g 'order' $null)) { $problems.Add("gate '$id': no order") }
    if (-not (Get-PackProp $g 'script' '')) { $problems.Add("gate '$id': no script") }
    if (-not (Get-PackProp $g 'title' '')) { $problems.Add("gate '$id': no title") }
    $est = Get-PackProp $g 'est_minutes' $null
    if ($null -eq $est -or $null -eq (Get-PackProp $est 'low' $null) -or $null -eq (Get-PackProp $est 'mid' $null)) { $problems.Add("gate '$id': est_minutes needs low and mid") }
    $needs = Get-PackProp $g 'needs' $null
    if ($grp -eq 'cold' -and $null -eq (Get-PackProp $needs 'boot_within_min' $null)) { $problems.Add("gate '$id': a cold gate needs needs.boot_within_min") }
    if ($grp -eq 'settled' -and $null -eq (Get-PackProp $needs 'uptime_min' $null)) { $problems.Add("gate '$id': a settled gate needs needs.uptime_min") }
    foreach ($l in @(Get-PackProp $g 'ledger' @())) {
      if (-not (Get-PackProp $l 'match' '')) { $problems.Add("gate '$id': a ledger entry has no match text") }
      $t = [string](Get-PackProp $l 'table' '')
      if (@('gates', 'open') -notcontains $t) { $problems.Add("gate '$id': ledger table '$t' is not gates or open") }
      $k = [string](Get-PackProp $l 'kind' '')
      if (@('verdict', 'info') -notcontains $k) { $problems.Add("gate '$id': ledger kind '$k' is not verdict or info") }
    }
  }
  foreach ($g in $gates) {
    foreach ($d in @(Get-PackProp $g 'depends_on' @())) {
      if (-not $ids.ContainsKey([string]$d)) { $problems.Add("gate '$(Get-PackProp $g 'id' '?')': depends_on '$d' is not a gate") }
    }
  }
  @($problems.ToArray())
}

function Read-PackManifest {
  param([Parameter(Mandatory = $true)] [string]$Path)
  $m = Read-PackJson -Path $Path
  if ($null -eq $m) { throw "The gate registry $Path is missing or empty." }
  $problems = @(Get-PackManifestProblems -Manifest $m)
  if ($problems.Count -gt 0) { throw ("The gate registry $Path is not valid: " + ($problems -join '; ')) }
  $m
}

# The two tables of docs/GATES.md as rows of cells: the gate ledger ('gates': Phase | Gate item |
# Machine or profile | Status | Evidence | Date) and the open items ('open': Found in | Belongs to |
# Item | Status | Evidence | Date). Rows are found by the heading above each table.
function Get-PackLedgerRows {
  param([Parameter(Mandatory = $true)] [string]$GatesMdPath)
  $rows = New-Object System.Collections.Generic.List[object]
  $table = $null
  $lineNo = 0
  foreach ($line in (Get-Content -LiteralPath $GatesMdPath -Encoding UTF8)) {
    $lineNo++
    if ($line -match '^##\s+Gates\s*$') { $table = 'gates'; continue }
    if ($line -match '^##\s+Open items\s*$') { $table = 'open'; continue }
    if ($line -match '^##\s+') { $table = $null; continue }
    if (-not $table) { continue }
    if ($line -notmatch '^\|') { continue }
    if ($line -match '^\|\s*(-+\s*\|\s*)+$') { continue }
    # Cells are separated by " | "; a literal pipe inside a cell is written \| in this file.
    $inner = $line.Trim().Trim('|')
    $cells = @([regex]::Split($inner, '(?<!\\)\|') | ForEach-Object { $_.Trim() })
    if ($cells.Count -lt 6) { continue }
    if ($cells[0] -eq 'Phase' -or $cells[0] -eq 'Found in') { continue }
    $rows.Add([pscustomobject]@{ Table = $table; Line = $lineNo; Cells = $cells; Raw = $line })
  }
  @($rows.ToArray())
}

# For each ledger entry of the manifest: how many rows of docs/GATES.md contain its match text in
# the item cell (cell 1 of the gates table, cell 2 of the open items). Exactly one is correct.
function Test-PackLedgerMatches {
  param([Parameter(Mandatory = $true)] $Manifest, [Parameter(Mandatory = $true)] [string]$GatesMdPath)
  $rows = @(Get-PackLedgerRows -GatesMdPath $GatesMdPath)
  $out = New-Object System.Collections.Generic.List[object]
  foreach ($g in @(Get-PackProp $Manifest 'gates' @())) {
    foreach ($l in @(Get-PackProp $g 'ledger' @())) {
      $table = [string](Get-PackProp $l 'table' '')
      $match = [string](Get-PackProp $l 'match' '')
      $cellIndex = $(if ($table -eq 'gates') { 1 } else { 2 })
      $hits = @($rows | Where-Object { $_.Table -eq $table -and $_.Cells[$cellIndex].IndexOf($match, [StringComparison]::Ordinal) -ge 0 })
      $out.Add([pscustomobject]@{ Gate = [string](Get-PackProp $g 'id' ''); Table = $table; Match = $match; Count = $hits.Count; Lines = @($hits | ForEach-Object { $_.Line }) })
    }
  }
  @($out.ToArray())
}

# ---- Boot time and uptime ------------------------------------------------------------------------

# The last full start of the kernel: the later of Windows's LastBootUpTime and the last event 12 of
# Microsoft-Windows-Kernel-General. With Fast Startup on, LastBootUpTime can stay at an older value
# across a shutdown (a restart is a full start, a shutdown is not).
function Get-PackBootInfo {
  $boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
  try {
    $ev = Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-General'; Id = 12 } -MaxEvents 1 -ErrorAction Stop
    if ($ev.TimeCreated -gt $boot) { $boot = $ev.TimeCreated }
  } catch { }
  [pscustomobject]@{ BootTime = $boot; UptimeMin = ((Get-Date) - $boot).TotalMinutes }
}

# ---- The rule that picks the next gate -----------------------------------------------------------

$script:DoneStatuses = @('PASS', 'FAIL', 'INFO', 'SKIPPED')

function Get-PackGateState {
  param($State, [string]$Id)
  $gs = Get-PackProp $State 'gates' $null
  if ($null -eq $gs) { return $null }
  Get-PackProp $gs $Id $null
}

# Pure: given the registry, the state, the uptime and what is true of the room right now, says what
# to run next and what is waiting for what. Facts: interface, ac, idle (each $true, $false or $null
# when unknown; unknown counts as missing for interface, as fine for the rest).
#   Next          the gate to run now, or $null
#   Waiting       gates that cannot run now, with the reason
#   NeedsRestart  cold gates are pending and the machine has been up longer than their window
#   WaitUptime    settled gates pending and the minutes still to wait
#   NotInPack     registry entries whose script is not in this pack
function Select-PackNextGate {
  param(
    [Parameter(Mandatory = $true)] $Manifest,
    $State = $null,
    [double]$UptimeMin = 0,
    [hashtable]$Facts = @{},
    [string[]]$Only = @(),
    [string[]]$Skip = @(),
    [string[]]$InPack = $null,
    [switch]$Retry,
    [switch]$Unattended,
    [switch]$RestartAllowed,
    [int]$MaxAttempts = 3
  )
  $waiting = New-Object System.Collections.Generic.List[object]
  $notInPack = New-Object System.Collections.Generic.List[string]
  $candidates = New-Object System.Collections.Generic.List[object]
  $needsRestart = New-Object System.Collections.Generic.List[string]
  $waitUptime = New-Object System.Collections.Generic.List[object]
  $factOf = { param($k) if ($Facts.ContainsKey($k)) { $Facts[$k] } else { $null } }

  foreach ($g in @(Get-PackProp $Manifest 'gates' @())) {
    $id = [string](Get-PackProp $g 'id' '')
    if ($Only.Count -gt 0 -and $Only -notcontains $id) { continue }
    if ($Skip -contains $id) { continue }
    if ($null -ne $InPack -and $InPack -notcontains $id) { $notInPack.Add($id); continue }
    $gs = Get-PackGateState -State $State -Id $id
    $status = [string](Get-PackProp $gs 'status' '')
    $attempts = [int](Get-PackProp $gs 'attempts' 0)
    $done = $false
    if ($script:DoneStatuses -contains $status) { $done = -not ($Retry -and $status -eq 'FAIL') }
    if ($status -eq 'INVALID') { $done = ($attempts -ge $MaxAttempts -and -not $Retry) }
    # A gate the person stopped or that died (interrupted) is run again when they name it with -Only: asking for it by
    # name is the answer to "try again", and an interrupted gate is not repeated within one run (it is no longer interrupted).
    if ($status -eq 'interrupted') { $done = ($attempts -ge $MaxAttempts -and -not $Retry -and ($Only -notcontains $id)) }
    if ($done) { continue }

    $needs = Get-PackProp $g 'needs' $null
    $why = New-Object System.Collections.Generic.List[string]
    $isInteractive = [bool](Get-PackProp $g 'interactive' $false)
    if ($Unattended -and $isInteractive) { $why.Add('needs a person at the keyboard (running unattended)') }
    if ([bool](Get-PackProp $needs 'interface' $false)) {
      $f = & $factOf 'interface'
      if ($f -ne $true) { $why.Add('needs the audio interface with its loopback cable (or VB-CABLE) connected') }
    }
    if ([bool](Get-PackProp $needs 'ac' $false)) {
      $f = & $factOf 'ac'
      if ($f -eq $false) { $why.Add('needs the laptop on mains power (AC)') }
    }
    if ([bool](Get-PackProp $needs 'mic' $false)) {
      $f = & $factOf 'mic'
      if ($f -eq $false) { $why.Add('needs a built-in or connected microphone') }
    }
    $window = Get-PackProp $needs 'boot_within_min' $null
    $windowPassed = $false
    if ($null -ne $window -and $UptimeMin -gt [double]$window) {
      $why.Add(("needs a fresh restart: up {0:n0} min, this gate starts only within {1} min of a restart" -f $UptimeMin, $window))
      $windowPassed = $true
    }
    $minUp = Get-PackProp $needs 'uptime_min' $null
    if ($null -ne $minUp -and $UptimeMin -lt [double]$minUp) {
      $left = [double]$minUp - $UptimeMin
      $why.Add(("needs the laptop up for {0} min: {1:n0} min to go" -f $minUp, $left))
      $waitUptime.Add([pscustomobject]@{ Id = $id; MinutesLeft = $left })
    }
    if ($null -ne $minUp -and [bool](Get-PackProp $needs 'idle' $false)) {
      $f = & $factOf 'idle'
      if ($f -eq $false) { $why.Add('needs an idle laptop (something is using the processor)') }
    }
    $dep = @(Get-PackProp $g 'depends_on' @())
    foreach ($d in $dep) {
      $ds = Get-PackGateState -State $State -Id ([string]$d)
      $dstat = [string](Get-PackProp $ds 'status' '')
      if (@('PASS', 'INFO', 'FAIL') -notcontains $dstat) { $why.Add("waits for gate '$d' (status: $(if ($dstat) { $dstat } else { 'not run' }))") }
    }
    if ($why.Count -gt 0) {
      # A restart is asked for only when it is all that is missing: no point restarting a laptop that
      # has no interface connected yet.
      if ($windowPassed -and $why.Count -eq 1) { $needsRestart.Add($id) }
      $waiting.Add([pscustomobject]@{ Id = $id; Reasons = @($why.ToArray()); Gate = $g })
      continue
    }
    $candidates.Add($g)
  }

  # Order: prepare gates first; then a gate that must follow a restart closely (it is ready now, so
  # it goes before the rest); then priority; then the order in the registry.
  $rank = {
    param($g)
    $prep = $(if ([bool](Get-PackProp $g 'prepare' $false)) { 0 } else { 1 })
    $needs = Get-PackProp $g 'needs' $null
    $bw = Get-PackProp $needs 'boot_within_min' $null
    # Gates that must follow a restart closely go first, the one with the shortest window first.
    $win = $(if ($null -ne $bw) { '0-{0:D4}' -f [int]$bw } else { '1-0000' })
    $pri = [int]([string](Get-PackProp $g 'priority' 'P3')).Substring(1)
    [pscustomobject]@{ Gate = $g; Key = ('{0}-{1}-{2}-{3:D5}' -f $prep, $win, $pri, [int](Get-PackProp $g 'order' 0)) }
  }
  $ranked = @($candidates | ForEach-Object { & $rank $_ } | Sort-Object Key)
  $next = $null
  if ($ranked.Count -gt 0) {
    $next = $ranked[0].Gate
    # A restart that would help takes precedence over any gate that is not a prepare gate: starting a
    # long gate first would let the restart window pass for good.
    if ($needsRestart.Count -gt 0 -and $RestartAllowed -and -not [bool](Get-PackProp $next 'prepare' $false)) { $next = $null }
  }
  [pscustomobject]@{
    Next = $next
    Waiting = @($waiting.ToArray())
    NeedsRestart = ($needsRestart.Count -gt 0)
    RestartFor = @($needsRestart.ToArray())
    WaitUptime = @($waitUptime.ToArray())
    NotInPack = @($notInPack.ToArray())
  }
}

# ---- State ---------------------------------------------------------------------------------------

function New-PackState {
  param([string]$Role = '', [string]$Profile = 'laptop', [bool]$Quick = $false)
  [ordered]@{
    pack_version = 1
    role = $Role
    profile = $Profile
    quick = $Quick
    created = (Get-Date).ToString('o')
    restart = [ordered]@{ requested_at = $null; boot_time_before = $null; resumed_at = $null; uptime_at_resume_min = $null }
    gates = [ordered]@{}
    order_used = New-Object System.Collections.ArrayList
  }
}

function Get-PackStatePath { param([string]$WorkDir) Join-Path $WorkDir 'state.json' }

function Read-PackState {
  param([string]$WorkDir)
  $p = Get-PackStatePath $WorkDir
  $s = Read-PackJson -Path $p -AsHashtable
  if ($null -eq $s) { return $null }
  if (-not $s.Contains('gates')) { $s['gates'] = [ordered]@{} }
  if (-not $s.Contains('order_used')) { $s['order_used'] = New-Object System.Collections.ArrayList }
  $s
}

function Save-PackState {
  param([string]$WorkDir, $State)
  Write-PackJson -Path (Get-PackStatePath $WorkDir) -Value $State
}

function Set-PackGateStatus {
  param([Parameter(Mandatory = $true)] $State, [Parameter(Mandatory = $true)] [string]$Id, [hashtable]$Fields)
  if (-not $State['gates'].Contains($Id)) { $State['gates'][$Id] = [ordered]@{ attempts = 0 } }
  foreach ($k in $Fields.Keys) { $State['gates'][$Id][$k] = $Fields[$k] }
}

# ---- Gate results --------------------------------------------------------------------------------

$script:ResultStatuses = @('PASS', 'FAIL', 'INVALID', 'INFO', 'PENDING', 'SKIPPED')

# What a gate script hands back. Status: PASS or FAIL (a verdict against a threshold), INVALID (no
# verdict, repeat it), INFO (numbers only), PENDING (could not run yet, with the reason), SKIPPED.
function New-PackResult {
  param(
    [Parameter(Mandatory = $true)] [string]$Id,
    [Parameter(Mandatory = $true)] [string]$Status,
    [string]$Headline = '',
    $Measured = $null,
    [string]$Threshold = '',
    [string]$Rig = '',
    [string]$RigDetail = '',
    [string[]]$Evidence = @(),
    [string[]]$Notes = @(),
    [bool]$Quick = $false,
    [bool]$NotAtFullLength = $false,
    $SharedBreaks = $null
  )
  if ($script:ResultStatuses -notcontains $Status) { throw "Unknown gate status '$Status'" }
  [ordered]@{
    id = $Id
    status = $Status
    headline = $Headline
    measured = $Measured
    threshold = $Threshold
    rig = $Rig
    rig_detail = $RigDetail
    shared_breaks = $SharedBreaks
    evidence = @($Evidence)
    notes = @($Notes)
    quick = $Quick
    not_at_full_length = $NotAtFullLength
    finished = (Get-Date).ToString('o')
  }
}

function Write-PackResult {
  param([Parameter(Mandatory = $true)] [string]$Path, [Parameter(Mandatory = $true)] $Result)
  Write-PackJson -Path $Path -Value $Result
}

# Reads the verdict of a legacy gate's evidence file (the "verdict:" line written by recorder-gate.ps1
# and kill-gate.ps1, and by the other gate scripts) as PASS, FAIL, INVALID or $null.
function Get-PackEvidenceVerdict {
  param([Parameter(Mandatory = $true)] [string]$Path)
  if (-not (Test-Path -LiteralPath $Path)) { return $null }
  foreach ($l in (Get-Content -LiteralPath $Path -Encoding UTF8 -TotalCount 80)) {
    if ($l -match '^verdict:\s*(PASS|FAIL|INVALID)') { return $Matches[1] }
  }
  $null
}

# Exit code of the legacy gate scripts to a status: 0 PASS (or a diagnostic), 1 FAIL, 3 INVALID.
function ConvertTo-PackStatus {
  param([int]$ExitCode)
  switch ($ExitCode) { 0 { 'PASS' } 1 { 'FAIL' } 3 { 'INVALID' } 2 { 'INVALID' } default { 'INVALID' } }
}

# ---- Facts about the laptop ----------------------------------------------------------------------

function Test-PackAdmin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# $true on mains power, $false on battery, $null when the machine has no battery (a desktop).
function Get-PackMainsPower {
  try {
    $b = @(Get-CimInstance Win32_Battery -ErrorAction Stop)
    if ($b.Count -eq 0) { return $null }
    # BatteryStatus 1 = discharging (on battery); everything else is mains or charging.
    return -not (@($b | Where-Object { [int]$_.BatteryStatus -eq 1 }).Count -gt 0)
  } catch { return $null }
}

# Average processor use over a few seconds; $true when below the limit. Unknown (the counter is not
# there) is $null, which the rule treats as fine.
function Test-PackIdle {
  param([double]$LimitPercent = 20, [int]$Seconds = 8)
  try {
    $s = @(Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples $Seconds -ErrorAction Stop)
    $avg = ($s | ForEach-Object { $_.CounterSamples[0].CookedValue } | Measure-Object -Average).Average
    return ([double]$avg -lt $LimitPercent)
  } catch { return $null }
}

# Programs that must be closed before the recorder gates (they steal the processor or sit in the audio
# path). The gate scripts refuse on their own; this tells the person at the start. -Names is for tests.
function Get-PackBlockingPrograms {
  param([string[]]$Names = $null)
  if ($null -eq $Names) { $Names = @(Get-Process -ErrorAction SilentlyContinue | ForEach-Object { $_.ProcessName }) }
  $rx = '^(LatMon|LatencyMon|RTSS|RTSSHooksLoader.*|MSIAfterburner|Steam|steamwebhelper|NahimicService|NahimicSvc.*|NahimicSmartAudio|A-Volute.*|AVolute.*|WavesSvc.*|DolbyDAX.*|SonicStudio.*)$'
  @($Names | Where-Object { $_ -match $rx } | Sort-Object -Unique)
}

# ---- Starting a gate -----------------------------------------------------------------------------

function ConvertTo-PackArg {
  param([string]$Value)
  if ($Value -match '[\s"]') { return '"' + ($Value -replace '"', '\"') + '"' }
  $Value
}

# Runs a gate script in its own Windows PowerShell 5.1 process, output to a log file, and waits.
# The gate script gets -CtxFile and -ResultFile and must write the result file. Returns the process
# exit code and whether it timed out. Never throws for a failing gate.
function Invoke-PackGateProcess {
  param(
    [Parameter(Mandatory = $true)] [string]$ScriptPath,
    [Parameter(Mandatory = $true)] [string]$CtxFile,
    [Parameter(Mandatory = $true)] [string]$ResultFile,
    [Parameter(Mandatory = $true)] [string]$LogFile,
    [int]$TimeoutMinutes = 240,
    [scriptblock]$OnTick = $null
  )
  $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $argList = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', (ConvertTo-PackArg $ScriptPath), '-CtxFile', (ConvertTo-PackArg $CtxFile), '-ResultFile', (ConvertTo-PackArg $ResultFile))
  $errFile = "$LogFile.err"
  $p = Start-Process -FilePath $ps -ArgumentList $argList -PassThru -WindowStyle Hidden -RedirectStandardOutput $LogFile -RedirectStandardError $errFile
  $null = $p.Handle
  $deadline = (Get-Date).AddMinutes($TimeoutMinutes)
  $timedOut = $false
  while (-not $p.HasExited) {
    if ((Get-Date) -gt $deadline) { $timedOut = $true; break }
    Start-Sleep -Seconds 2
    if ($OnTick) { & $OnTick $p }
  }
  if ($timedOut) {
    $null = Invoke-PackNative -FilePath 'taskkill.exe' -ArgumentList @('/F', '/T', '/PID', "$($p.Id)") -TimeoutSeconds 30
    return [pscustomobject]@{ ExitCode = -1; TimedOut = $true }
  }
  $p.WaitForExit()
  [pscustomobject]@{ ExitCode = [int]$p.ExitCode; TimedOut = $false }
}

# ---- Unblock and launch checks --------------------------------------------------------------------

# Removes the "downloaded from the internet" mark from every file of the pack (what Windows adds to
# files that came by zip, mail or a browser). Never touches protection settings. Returns what is
# still marked afterwards (a file system without alternate streams has none).
function Invoke-PackUnblock {
  param([Parameter(Mandatory = $true)] [string]$Root)
  $marked = 0
  $still = New-Object System.Collections.Generic.List[string]
  foreach ($f in (Get-ChildItem -LiteralPath $Root -Recurse -File -Force -ErrorAction SilentlyContinue)) {
    if ($f.Extension -match '^\.(bin|wav|json|txt|md|csv)$') { continue }
    $has = $false
    try { $has = $null -ne (Get-Item -LiteralPath $f.FullName -Stream 'Zone.Identifier' -ErrorAction SilentlyContinue) } catch { }
    if (-not $has) { continue }
    $marked++
    try { Unblock-File -LiteralPath $f.FullName -ErrorAction Stop } catch { }
    $left = $false
    try { $left = $null -ne (Get-Item -LiteralPath $f.FullName -Stream 'Zone.Identifier' -ErrorAction SilentlyContinue) } catch { }
    if ($left) { $still.Add($f.FullName) }
  }
  [pscustomobject]@{ Unblocked = $marked - $still.Count; StillMarked = @($still.ToArray()) }
}

# Starts an executable with a harmless argument and reports plainly whether Windows let it run. A
# block by Microsoft Defender, SmartScreen or a policy is reported with the file; nothing about
# protection is changed here or anywhere else in the pack.
function Test-PackLaunchable {
  param([Parameter(Mandatory = $true)] [string]$Path, [string[]]$Arguments = @('--version'), [int]$TimeoutSeconds = 30)
  if (-not (Test-Path -LiteralPath $Path)) { return [pscustomobject]@{ Path = $Path; Ok = $false; Blocked = $false; Message = 'the file is not there' } }
  try {
    $p = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru -WindowStyle Hidden -RedirectStandardOutput ([System.IO.Path]::GetTempFileName()) -RedirectStandardError ([System.IO.Path]::GetTempFileName())
    if (-not $p.WaitForExit($TimeoutSeconds * 1000)) { try { $p.Kill() } catch { } }
    return [pscustomobject]@{ Path = $Path; Ok = $true; Blocked = $false; Message = 'started' }
  } catch {
    $m = $_.Exception.Message
    # 225 = the file contains a virus or potentially unwanted software; 1260 = blocked by policy; 5 = access denied.
    $blocked = ($m -match 'virus|potentially unwanted|blocked|administrator has blocked|Access is denied|group policy|SmartScreen|Defender')
    return [pscustomobject]@{ Path = $Path; Ok = $false; Blocked = $blocked; Message = $m }
  }
}

# ---- Resume after a restart ----------------------------------------------------------------------

# The two environment variables let the tests use a harmless task name and Run key; a real run never sets them.
$script:ResumeTaskName = $(if ($env:WB_PACK_RESUME_TASK) { $env:WB_PACK_RESUME_TASK } else { 'STELMAR-GatePack-Resume' })
$script:RunKey = $(if ($env:WB_PACK_RESUME_RUNKEY) { $env:WB_PACK_RESUME_RUNKEY } else { 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' })

# Windows' own shutdown.exe. WB_PACK_SHUTDOWN lets a test point at a stub so no test can restart the laptop.
function Get-PackShutdownExe {
  if ($env:WB_PACK_SHUTDOWN) { return $env:WB_PACK_SHUTDOWN }
  Join-Path $env:SystemRoot 'System32\shutdown.exe'
}

# schtasks.exe writes to stderr when a task is missing; under -ErrorAction Stop that would end the
# caller, so the call is made with its own Continue preference.
function Invoke-PackSchtasks {
  param([string[]]$Arguments)
  $r = Invoke-PackNative -FilePath 'schtasks.exe' -ArgumentList $Arguments -TimeoutSeconds 60
  [pscustomobject]@{ ExitCode = $r.ExitCode; Output = ((@($r.Output) + @($r.Error)) -join ' ') }
}

# The task that starts the runner again when this person logs in after the restart.
#
# The task never carries the runner's own command line. schtasks.exe /TR refuses more than 261
# characters, and the real command (run-gates.ps1 with its pack, work, lock, backup and emulator paths and
# the gate list) is longer: "Value for '/TR' option cannot be more than 261 character(s)" stopped the
# dry-run restart on 17 Nov 2026. The command lives in a small launcher script in the work folder; the task
# runs only "powershell.exe -File <launcher>".

# Pure. One value as PowerShell source: strings single-quoted (a quote doubled), booleans, arrays.
function ConvertTo-PackPsLiteral {
  param($Value)
  if ($null -eq $Value) { return '$null' }
  if ($Value -is [bool]) { return $(if ($Value) { '$true' } else { '$false' }) }
  if ($Value -is [System.Management.Automation.SwitchParameter]) { return $(if ($Value.IsPresent) { '$true' } else { '$false' }) }
  if ($Value -is [double] -or $Value -is [int] -or $Value -is [long]) { return ([string]::Format([Globalization.CultureInfo]::InvariantCulture, '{0}', $Value)) }
  if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [string]) {
    return '@(' + ((@($Value) | ForEach-Object { ConvertTo-PackPsLiteral $_ }) -join ', ') + ')'
  }
  "'" + (([string]$Value) -replace "'", "''") + "'"
}

# Pure. The parameters the resumed runner gets, as an ordered hashtable (only what is set). Emulator,
# role and the gate list are the ones of the run that asked for the restart.
function New-PackResumeParams {
  param(
    [string]$Profile = 'laptop', [string]$PackRoot, [string]$WorkDir, [string]$LockDir = '', [string]$BackupRoot = '',
    [string]$Emulate = '', [bool]$Quick = $false, [string[]]$Only = @(), [string[]]$Skip = @(), [string]$Role = '',
    [double]$MinFreeRamGB = 0, [string]$RunLabel = '', [bool]$NoZip = $false
  )
  $p = [ordered]@{ Resumed = $true; Profile = $Profile; PackRoot = $PackRoot; WorkDir = $WorkDir }
  if ($Role) { $p['Role'] = $Role }
  if ($LockDir) { $p['LockDir'] = $LockDir }
  if ($BackupRoot) { $p['BackupRoot'] = $BackupRoot }
  if ($Emulate) { $p['Emulate'] = $Emulate }
  if ($Quick) { $p['Quick'] = $true }
  if (@($Only).Count -gt 0) { $p['Only'] = @($Only) }
  if (@($Skip).Count -gt 0) { $p['Skip'] = @($Skip) }
  if ($MinFreeRamGB -gt 0) { $p['MinFreeRamGB'] = $MinFreeRamGB }
  if ($RunLabel) { $p['RunLabel'] = $RunLabel }
  if ($NoZip) { $p['NoZip'] = $true }
  $p
}

# The drive roots among Paths that Windows reports as removable (a USB stick; a USB SSD is usually 'Fixed').
function Get-PackRemovablePaths {
  param([string[]]$Paths)
  $out = New-Object System.Collections.Generic.List[string]
  foreach ($p in @($Paths)) {
    try {
      $root = [System.IO.Path]::GetPathRoot($p)
      if ($root -and ([System.IO.DriveInfo]$root).DriveType -eq [System.IO.DriveType]::Removable -and -not $out.Contains($root)) { $out.Add($root) }
    } catch { }
  }
  @($out.ToArray())
}

function Get-PackResumeLauncherPath {
  param([Parameter(Mandatory = $true)] [string]$WorkDir)
  Join-Path $WorkDir 'resume-pack.ps1'
}

# Writes the launcher: sets the work folder as the current folder, runs the runner with the parameters
# (splatted, so no quoting of paths is needed), and keeps the window open at the end so the person
# reads the final banner. An error is shown, written to resume-error.txt and waited on, never lost.
function New-PackResumeLauncher {
  param([Parameter(Mandatory = $true)] [string]$Path, [Parameter(Mandatory = $true)] [string]$RunScript, [Parameter(Mandatory = $true)] [string]$WorkDir, [Parameter(Mandatory = $true)] $Params, [int]$WaitForRunnerSeconds = 120)
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add('# STELMAR gate pack: carries on after the restart. Written by run-gates.ps1; removed when the run ends.')
  $lines.Add("`$ErrorActionPreference = 'Stop'")
  $lines.Add('$workDir = ' + (ConvertTo-PackPsLiteral $WorkDir))
  $lines.Add('$p = @{')
  foreach ($k in @($Params.Keys)) { $lines.Add(('  {0} = {1}' -f $k, (ConvertTo-PackPsLiteral $Params[$k]))) }
  $lines.Add('}')
  $lines.Add('try {')
  # A pack on a USB drive may not be ready 30 seconds after logon: wait for the runner to appear.
  $lines.Add('  $waited = 0')
  $lines.Add('  while (-not (Test-Path -LiteralPath ' + (ConvertTo-PackPsLiteral $RunScript) + ') -and $waited -lt ' + $WaitForRunnerSeconds + ') { Start-Sleep -Seconds 2; $waited += 2 }')
  $lines.Add('  Set-Location -LiteralPath $workDir')
  $lines.Add('  & ' + (ConvertTo-PackPsLiteral $RunScript) + ' @p')
  $lines.Add('} catch {')
  $lines.Add('  $m = "The pack could not carry on after the restart: " + $_.Exception.Message')
  $lines.Add('  try { Set-Content -LiteralPath (Join-Path $workDir ''resume-error.txt'') -Value $m } catch { }')
  $lines.Add('  Write-Host $m -ForegroundColor Red')
  $lines.Add('}')
  $lines.Add("Read-Host 'This window can be closed now. Press Enter to close it' | Out-Null")
  $dir = Split-Path -Parent $Path
  if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  # UTF-8 with a byte-order mark, so Windows PowerShell 5.1 reads a folder name with accents correctly.
  [System.IO.File]::WriteAllText($Path, (($lines.ToArray()) -join "`r`n") + "`r`n", (New-Object System.Text.UTF8Encoding($true)))
  $Path
}

function Get-PackPowerShellExe { Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe' }

# Registers the resume at logon of the current user and reads it back. Returns a description. Throws (and
# leaves nothing behind) when the task cannot be created or is not what was asked for. With administrator
# rights it is a scheduled task at logon, highest run level, started 30 seconds after logon so Windows
# start-up settles, with no time limit and no stop on battery; without them it is a Run key entry (not
# elevated; the cold-cache gates need administrator rights and say so). -TaskName, -RunKey and -RunLevel
# exist so tests can use harmless names.
function Register-PackResume {
  param(
    [Parameter(Mandatory = $true)] [string]$Launcher,
    [bool]$Admin = $true,
    [string]$TaskName = $script:ResumeTaskName,
    [string]$RunKey = $script:RunKey,
    [ValidateSet('Highest', 'Limited')] [string]$RunLevel = 'Highest',
    [int]$DelaySeconds = 30
  )
  if (-not (Test-Path -LiteralPath $Launcher)) { throw "Could not register the logon task: the launcher $Launcher is missing" }
  $ps = Get-PackPowerShellExe
  $argText = '-NoProfile -ExecutionPolicy Bypass -File "{0}"' -f $Launcher
  $work = Split-Path -Parent $Launcher
  if (-not $Admin) {
    $cmd = '"{0}" {1}' -f $ps, $argText
    # The Run key's command line is read by Windows with a limit of about 260 characters.
    if ($cmd.Length -gt 255) { throw "Could not register the carry-on after the restart: the launcher path is too long for the Run key ($($cmd.Length) characters, 255 allowed). Use a shorter work folder." }
    if (-not (Test-Path -LiteralPath $RunKey)) { New-Item -Path $RunKey -Force | Out-Null }
    Set-ItemProperty -Path $RunKey -Name $TaskName -Value $cmd -ErrorAction Stop
    $back = (Get-ItemProperty -Path $RunKey -Name $TaskName -ErrorAction Stop).$TaskName
    if ($back -ne $cmd) { Remove-ItemProperty -Path $RunKey -Name $TaskName -ErrorAction SilentlyContinue; throw 'Could not register the carry-on after the restart: the Run key entry reads back different.' }
    return 'Run key entry ' + $TaskName + ' (starts at logon, not elevated)'
  }
  $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
  $how = ''
  try {
    $action = New-ScheduledTaskAction -Execute $ps -Argument $argText -WorkingDirectory $work
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
    $trigger.Delay = 'PT{0}S' -f $DelaySeconds
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel $RunLevel
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
    $null = Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force -ErrorAction Stop
    $how = 'scheduled task ' + $TaskName
  } catch {
    # The Task Scheduler cmdlets are missing or refused: try schtasks.exe, whose /TR holds only the launcher.
    $why = $_.Exception.Message
    $tr = '"{0}" {1}' -f $ps, $argText
    if ($tr.Length -gt 255) { throw "Could not register the logon task: $why (and the launcher path is too long for schtasks: $($tr.Length) characters, 255 allowed)" }
    $lvl = $(if ($RunLevel -eq 'Highest') { 'HIGHEST' } else { 'LIMITED' })
    $r = Invoke-PackSchtasks -Arguments @('/Create', '/F', '/TN', $TaskName, '/SC', 'ONLOGON', '/RL', $lvl, '/DELAY', ('0000:{0:00}' -f [math]::Min(59, $DelaySeconds)), '/TR', $tr)
    if ($r.ExitCode -ne 0) { throw "Could not register the logon task: $why; schtasks said: $($r.Output)" }
    $how = 'scheduled task ' + $TaskName + ' (through schtasks)'
  }
  # Read it back: it must be there and point at the launcher.
  $q = Invoke-PackSchtasks -Arguments @('/Query', '/TN', $TaskName, '/XML')
  if ($q.ExitCode -ne 0 -or $q.Output -notmatch [regex]::Escape((Split-Path -Leaf $Launcher))) {
    $null = Invoke-PackSchtasks -Arguments @('/Delete', '/F', '/TN', $TaskName)
    throw "Could not register the logon task: it was created but does not read back as the launcher ($($q.Output))"
  }
  $how
}

# Removes the task and the Run key entry, and the launcher when -WorkDir is given. Returns what was removed.
function Unregister-PackResume {
  param([string]$TaskName = $script:ResumeTaskName, [string]$RunKey = $script:RunKey, [string]$WorkDir = '')
  $removed = New-Object System.Collections.Generic.List[string]
  $q = Invoke-PackSchtasks -Arguments @('/Query', '/TN', $TaskName)
  if ($q.ExitCode -eq 0) {
    $null = Invoke-PackSchtasks -Arguments @('/Delete', '/F', '/TN', $TaskName)
    $removed.Add('scheduled task ' + $TaskName)
  }
  $v = $null
  if (Test-Path -LiteralPath $RunKey) { $v = Get-ItemProperty -Path $RunKey -Name $TaskName -ErrorAction SilentlyContinue }
  if ($v) {
    Remove-ItemProperty -Path $RunKey -Name $TaskName -ErrorAction SilentlyContinue
    $removed.Add('Run key entry ' + $TaskName)
  }
  if ($WorkDir) {
    $l = Get-PackResumeLauncherPath -WorkDir $WorkDir
    if (Test-Path -LiteralPath $l) { Remove-Item -LiteralPath $l -Force -ErrorAction SilentlyContinue; $removed.Add('launcher ' + $l) }
  }
  @($removed.ToArray())
}

function Test-PackResumeRegistered {
  param([string]$TaskName = $script:ResumeTaskName, [string]$RunKey = $script:RunKey)
  $q = Invoke-PackSchtasks -Arguments @('/Query', '/TN', $TaskName)
  if ($q.ExitCode -eq 0) { return $true }
  if (-not (Test-Path -LiteralPath $RunKey)) { return $false }
  [bool](Get-ItemProperty -Path $RunKey -Name $TaskName -ErrorAction SilentlyContinue)
}

# Asks Windows to restart in 15 seconds (shutdown /a cancels). Returns ExitCode and Output.
function Request-PackRestart {
  $r = Invoke-PackNative -FilePath (Get-PackShutdownExe) -ArgumentList @('/r', '/t', '15', '/c', 'STELMAR gate pack: restart for the cold-cache gates')
  [pscustomobject]@{ ExitCode = $r.ExitCode; Output = ((@($r.Output) + @($r.Error)) -join ' ') }
}

# ---- What the pack changes, and putting it back --------------------------------------------------

# Anything a gate changes on the laptop outside its own work folder is recorded here BEFORE it is
# changed, with the way to put it back. -Cleanup (and the end of every run) undoes them; the banner
# lists what could not be put back.
function Get-PackChangesPath { param([string]$WorkDir) Join-Path $WorkDir 'changes.json' }

function Add-PackChange {
  param(
    [Parameter(Mandatory = $true)] [string]$WorkDir,
    [Parameter(Mandatory = $true)] [ValidateSet('mic-mute', 'render-volume', 'power-timeouts', 'other')] [string]$Kind,
    [Parameter(Mandatory = $true)] [string]$Description,
    [hashtable]$Restore = @{}
  )
  $p = Get-PackChangesPath $WorkDir
  $list = Read-PackJson -Path $p -AsHashtable
  if ($null -eq $list) { $list = [ordered]@{ changes = (New-Object System.Collections.ArrayList) } }
  [void]$list['changes'].Add([ordered]@{ kind = $Kind; description = $Description; restore = $Restore; at = (Get-Date).ToString('o'); restored = $false; restore_note = '' })
  Write-PackJson -Path $p -Value $list
}

# Puts back every recorded change. -Testbench is the pack's wb-testbench (volume and mute). Returns
# lines for the banner: what was restored and what is still changed.
function Invoke-PackCleanup {
  param([Parameter(Mandatory = $true)] [string]$WorkDir, [string]$Testbench = '')
  $lines = New-Object System.Collections.Generic.List[string]
  foreach ($r in @(Unregister-PackResume -WorkDir $WorkDir)) { $lines.Add("removed: $r") }
  $p = Get-PackChangesPath $WorkDir
  $list = Read-PackJson -Path $p -AsHashtable
  if ($null -ne $list) {
    foreach ($c in @($list['changes'])) {
      if ([bool]$c['restored']) { continue }
      $ok = $false
      $note = ''
      $rest = $c['restore']
      try {
        switch ([string]$c['kind']) {
          'mic-mute' {
            if (-not $Testbench -or -not (Test-Path -LiteralPath $Testbench)) { throw 'wb-testbench is not available' }
            $res = Invoke-PackNative -FilePath $Testbench -ArgumentList @('mute', '--device', ([string]$rest['device']), '--state', ([string]$rest['state_before']))
            if ($res.ExitCode -ne 0) { throw ((@($res.Output) + @($res.Error)) -join ' ') }
            $ok = $true
          }
          'render-volume' {
            if (-not $Testbench -or -not (Test-Path -LiteralPath $Testbench)) { throw 'wb-testbench is not available' }
            $res = Invoke-PackNative -FilePath $Testbench -ArgumentList @('volume', '--device', ([string]$rest['device']), '--flow', 'render', '--level', ([string]$rest['level_before']))
            if ($res.ExitCode -ne 0) { throw ((@($res.Output) + @($res.Error)) -join ' ') }
            $ok = $true
          }
          'power-timeouts' {
            foreach ($kv in @($rest['values'])) {
              $null = Invoke-PackNative -FilePath 'powercfg.exe' -ArgumentList @('/setacvalueindex', 'SCHEME_CURRENT', ([string]$kv['sub']), ([string]$kv['setting']), ([string]$kv['value']))
            }
            $null = Invoke-PackNative -FilePath 'powercfg.exe' -ArgumentList @('/setactive', 'SCHEME_CURRENT')
            $ok = $true
          }
          default { $note = 'nothing to undo automatically' ; $ok = $true }
        }
      } catch { $note = $_.Exception.Message }
      $c['restored'] = $ok
      $c['restore_note'] = $note
      if ($ok) { $lines.Add("restored: $($c['description'])") }
      else { $lines.Add("STILL CHANGED: $($c['description']) ($note)") }
    }
    Write-PackJson -Path $p -Value $list
  }
  if ($lines.Count -eq 0) { $lines.Add('nothing had been changed') }
  @($lines.ToArray())
}

# What is still changed according to the record (for the end banner and summary.json).
function Get-PackStillChanged {
  param([string]$WorkDir)
  $list = Read-PackJson -Path (Get-PackChangesPath $WorkDir) -AsHashtable
  $out = New-Object System.Collections.Generic.List[string]
  if ($null -ne $list) { foreach ($c in @($list['changes'])) { if (-not [bool]$c['restored']) { $out.Add([string]$c['description']) } } }
  if (Test-PackResumeRegistered) { $out.Add('the logon task that resumes the pack after a restart') }
  @($out.ToArray())
}

# ---- Machine name, summary, zip ------------------------------------------------------------------

# Maker and model, with a short hash of the BIOS serial number so two laptops of one model can be told
# apart. The computer's own name is never used: it can carry a person's name.
function Get-PackMachineName {
  try {
    $cs = Get-CimInstance Win32_ComputerSystem
    $make = ([string]$cs.Manufacturer).Trim()
    $model = ([string]$cs.Model).Trim()
    $serial = [string](Get-CimInstance Win32_BIOS).SerialNumber
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $h = ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($serial)) | Select-Object -First 3 | ForEach-Object { $_.ToString('x2') }) -join ''
    $raw = "$make $model $h"
  } catch { $raw = 'unknown-machine' }
  ($raw -replace '[^A-Za-z0-9]+', '-').Trim('-')
}

function New-PackSummary {
  param(
    [Parameter(Mandatory = $true)] $Manifest,
    [Parameter(Mandatory = $true)] $State,
    [Parameter(Mandatory = $true)] [string]$Machine,
    [string]$Emulated = '',
    [string]$SourceCommit = '',
    [string[]]$InPack = $null,
    $Fingerprint = $null,
    [string[]]$StillChanged = @(),
    [string]$RunLabel = ''
  )
  $gates = New-Object System.Collections.ArrayList
  $pending = New-Object System.Collections.ArrayList
  $notInPack = New-Object System.Collections.ArrayList
  foreach ($g in @(Get-PackProp $Manifest 'gates' @())) {
    $id = [string](Get-PackProp $g 'id' '')
    if ($null -ne $InPack -and $InPack -notcontains $id) {
      [void]$notInPack.Add([ordered]@{ id = $id; priority = (Get-PackProp $g 'priority' ''); title = (Get-PackProp $g 'title' ''); reason = 'not in this pack'; ledger = @(Get-PackProp $g 'ledger' @()) })
      continue
    }
    $gs = Get-PackGateState -State $State -Id $id
    $status = [string](Get-PackProp $gs 'status' '')
    if (-not $status -or $status -eq 'PENDING' -or $status -eq 'interrupted' -or $status -eq 'running') {
      [void]$pending.Add([ordered]@{ id = $id; priority = (Get-PackProp $g 'priority' ''); title = (Get-PackProp $g 'title' ''); reason = [string](Get-PackProp $gs 'reason' 'not run'); ledger = @(Get-PackProp $g 'ledger' @()) })
      continue
    }
    $res = $null
    $rf = [string](Get-PackProp $gs 'result_file' '')
    if ($rf -and (Test-Path -LiteralPath $rf)) { $res = Read-PackJson -Path $rf -AsHashtable }
    $entry = [ordered]@{
      id = $id
      priority = (Get-PackProp $g 'priority' '')
      status = $status
      attempts = (Get-PackProp $gs 'attempts' 0)
      started = (Get-PackProp $gs 'started' $null)
      ended = (Get-PackProp $gs 'ended' $null)
      minutes = (Get-PackProp $gs 'minutes' $null)
      headline = $(if ($res) { $res['headline'] } else { [string](Get-PackProp $gs 'reason' '') })
      measured = $(if ($res) { $res['measured'] } else { $null })
      threshold = $(if ($res) { $res['threshold'] } else { (Get-PackProp $g 'threshold' '') })
      rig = $(if ($res) { $res['rig'] } else { '' })
      rig_detail = $(if ($res) { $res['rig_detail'] } else { '' })
      shared_breaks = $(if ($res) { $res['shared_breaks'] } else { $null })
      evidence = $(if ($res) { @($res['evidence']) } else { @() })
      notes = $(if ($res) { @($res['notes']) } else { @() })
      quick = [bool](Get-PackProp $State 'quick' $false)
      not_at_full_length = $(if ($res) { [bool]$res['not_at_full_length'] } else { $false })
      ledger = @(Get-PackProp $g 'ledger' @())
    }
    [void]$gates.Add($entry)
  }
  [ordered]@{
    pack_version = 1
    source_commit = $SourceCommit
    machine = $Machine
    role = (Get-PackProp $State 'role' '')
    emulated = $(if ($Emulated) { $Emulated } else { $null })
    quick = [bool](Get-PackProp $State 'quick' $false)
    run_label = $RunLabel
    created = (Get-Date).ToString('o')
    restart = (Get-PackProp $State 'restart' $null)
    order_used = @(Get-PackProp $State 'order_used' @())
    gates = @($gates.ToArray())
    pending = @($pending.ToArray())
    not_in_pack = @($notInPack.ToArray())
    still_changed = @($StillChanged)
    fingerprint = $Fingerprint
  }
}

# Files that go into the results zip: every small report, log, table and capability file under the
# work folder; never audio (a recording, a source file, a mirror), a model or a very large file.
function Get-PackCollectFiles {
  param([Parameter(Mandatory = $true)] [string]$WorkDir, [long]$MaxBytes = 64MB)
  $skipDirs = '\\(audio|source|backup|mirror|rec|reference|reference-wasapi|mic-keep|models|engine-[a-z0-9]+|engines|bin)(\\|$)'
  @(Get-ChildItem -LiteralPath $WorkDir -Recurse -File -Force -ErrorAction SilentlyContinue | Where-Object {
      $rel = $_.FullName.Substring($WorkDir.Length)
      $_.Length -le $MaxBytes -and $_.Extension -notmatch '^\.(wav|bin|zip|tmp|lock|exe|dll|pdb|sys)$' -and $rel -notmatch $skipDirs
    })
}

# Writes the zip with forward slashes in the entry names (the form Windows PowerShell 5.1's own
# Compress-Archive does not guarantee) and a ZIP-MANIFEST.json with each file's SHA-256 for ingest.
# Pure. The name of a run's results zip. Until 17 Nov 2026 it was gate-results-<machine>.zip for every run, so a
# second run on the same laptop with another work folder (the mic run has its own) rewrote the first run's zip. Now the
# name carries the moment the work folder's first run began (kept in state.json, so a restart or a resume writes the
# same name and a later run of the same work folder rewrites its own cumulative zip) and the run label if there is one.
function Get-PackZipName {
  [CmdletBinding()]
  param([string]$Machine, $Created, [string]$RunLabel = '')
  $when = $null
  if ($Created -is [datetime]) { $when = $Created }
  elseif ($Created) { try { $when = ([datetimeoffset]::Parse([string]$Created, [Globalization.CultureInfo]::InvariantCulture)).DateTime } catch { $when = $null } }
  if ($null -eq $when) { $when = Get-Date }
  $m = ($Machine -replace '[^A-Za-z0-9._-]', '-')
  if (-not $m) { $m = 'laptop' }
  $l = ($RunLabel -replace '[^A-Za-z0-9._-]', '-').Trim('-')
  if ($l.Length -gt 40) { $l = $l.Substring(0, 40) }
  'gate-results-{0}-{1}{2}.zip' -f $m, $when.ToString('yyyyMMdd-HHmmss', [Globalization.CultureInfo]::InvariantCulture), $(if ($l) { '-' + $l } else { '' })
}

function New-PackZip {
  param(
    [Parameter(Mandatory = $true)] [string]$WorkDir,
    [Parameter(Mandatory = $true)] [string]$ZipPath,
    [Parameter(Mandatory = $true)] $Summary,
    [string]$CommitText = ''
  )
  Add-Type -AssemblyName System.IO.Compression
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
  $files = @(Get-PackCollectFiles -WorkDir $WorkDir)
  $zip = [System.IO.Compression.ZipFile]::Open($ZipPath, [System.IO.Compression.ZipArchiveMode]::Create)
  $manifest = New-Object System.Collections.ArrayList
  try {
    $add = {
      param([string]$entryName, [byte[]]$bytes)
      $e = $zip.CreateEntry($entryName, [System.IO.Compression.CompressionLevel]::Optimal)
      $s = $e.Open()
      try { $s.Write($bytes, 0, $bytes.Length) } finally { $s.Dispose() }
    }
    foreach ($f in $files) {
      $rel = ($f.FullName.Substring($WorkDir.Length).TrimStart('\') -replace '\\', '/')
      $bytes = [System.IO.File]::ReadAllBytes($f.FullName)
      & $add ('work/' + $rel) $bytes
      $sha = [System.Security.Cryptography.SHA256]::Create()
      $hash = ($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join ''
      [void]$manifest.Add([ordered]@{ path = ('work/' + $rel); bytes = $bytes.Length; sha256 = $hash })
    }
    $summaryJson = $Summary | ConvertTo-Json -Depth 14
    & $add 'summary.json' ([Text.Encoding]::UTF8.GetBytes($summaryJson))
    $zm = [ordered]@{ pack_version = 1; source_commit = $CommitText; files = @($manifest.ToArray()) }
    & $add 'ZIP-MANIFEST.json' ([Text.Encoding]::UTF8.GetBytes(($zm | ConvertTo-Json -Depth 6)))
  } finally { $zip.Dispose() }
  [pscustomobject]@{ Path = $ZipPath; Files = $files.Count + 2; Bytes = (Get-Item -LiteralPath $ZipPath).Length }
}

# ---- Plain-words lines for the console ------------------------------------------------------------

function Format-PackMinutes {
  param([double]$Minutes)
  if ($Minutes -lt 60) { return ('{0:n0} min' -f $Minutes) }
  $h = [math]::Floor($Minutes / 60.0)
  $m = [math]::Round($Minutes - 60 * $h)
  if ($m -ge 60) { $h++; $m = 0 }
  if ($m -eq 0) { return ('{0:n0} h' -f $h) }
  '{0:n0} h {1:n0} min' -f $h, $m
}

# One line per gate for the banner after each gate and at the end.
function Format-PackGateLine {
  param([string]$Id, [string]$Status, [string]$Headline, [string]$Rig = '', $Minutes = $null)
  $r = $(if ($Rig) { " [rig: $Rig]" } else { '' })
  $m = $(if ($null -ne $Minutes) { " ({0})" -f (Format-PackMinutes ([double]$Minutes)) } else { '' })
  ('{0,-8} {1,-24}{2}{3} {4}' -f $Status, $Id, $m, $r, $Headline).TrimEnd()
}

Export-ModuleMember -Function Get-PackProp, ConvertTo-PackHashtable, Read-PackJson, Write-PackJson, Write-PackLog, Set-PackLogFile, Get-PackManifestProblems, Read-PackManifest, Get-PackLedgerRows, Test-PackLedgerMatches, Get-PackBootInfo, Select-PackNextGate, New-PackState, Get-PackStatePath, Read-PackState, Save-PackState, Set-PackGateStatus, Get-PackGateState, New-PackResult, Write-PackResult, Get-PackEvidenceVerdict, ConvertTo-PackStatus, Test-PackAdmin, Get-PackMainsPower, Test-PackIdle, Get-PackBlockingPrograms, ConvertTo-PackArg, Invoke-PackGateProcess, Invoke-PackUnblock, Test-PackLaunchable, ConvertTo-PackPsLiteral, New-PackResumeParams, Get-PackRemovablePaths, Get-PackResumeLauncherPath, New-PackResumeLauncher, Get-PackPowerShellExe, Get-PackShutdownExe, Request-PackRestart, Register-PackResume, Unregister-PackResume, Test-PackResumeRegistered, Get-PackChangesPath, Add-PackChange, Invoke-PackCleanup, Get-PackStillChanged, Get-PackMachineName, New-PackSummary, Get-PackCollectFiles, Get-PackZipName, New-PackZip, Format-PackMinutes, Format-PackGateLine, Write-PackBanner, New-PackPromptHandler
