<#
  Runs inside the emulation job (started by recorder-gate.ps1 through wb-emulate): starts the
  gate build of the app and, for the load test, the stress load in the same job; waits for the
  app to exit; writes the app's CPU time. Not meant to be run by hand.

  ASCII only; runs under Windows PowerShell 5.1.
#>
param(
  [Parameter(Mandatory = $true)] [string]$Exe,
  [Parameter(Mandatory = $true)] [string]$ResultFile,
  [string]$Testbench,
  [switch]$Stress,
  [string]$GpuName = 'UHD',
  [string]$StressStopFile,
  [string]$StressReport
)
$ErrorActionPreference = 'Stop'
$stressProc = $null
if ($Stress) {
  $a = @('stress', '--gpu', $GpuName, '--seconds', '14400', '--stop-file', $StressStopFile, '--report', $StressReport)
  $stressProc = Start-Process -FilePath $Testbench -ArgumentList ($a | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -PassThru -WindowStyle Hidden
}
$app = Start-Process -FilePath $Exe -PassThru
$app.WaitForExit()
$result = [ordered]@{
  app_pid            = $app.Id
  app_exit_code      = $app.ExitCode
  app_started        = $app.StartTime.ToString('o')
  app_exited         = $app.ExitTime.ToString('o')
  app_cpu_seconds    = $app.TotalProcessorTime.TotalSeconds
  app_wall_seconds   = ($app.ExitTime - $app.StartTime).TotalSeconds
  stress_pid         = $(if ($stressProc) { $stressProc.Id } else { $null })
}
if ($stressProc) {
  if (-not (Test-Path $StressStopFile)) { Set-Content -LiteralPath $StressStopFile -Value 'stop' }
  [void]$stressProc.WaitForExit(60000)
}
$result | ConvertTo-Json | Set-Content -LiteralPath $ResultFile -Encoding utf8
