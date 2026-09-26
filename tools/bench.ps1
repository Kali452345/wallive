# Benchmark mode (ADR-010): runs wallive with the given arguments, waits for
# warm-up, then samples the process for a fixed window and prints one result
# line. Measuring from outside keeps the always-running process free of PDH /
# WMI code.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/bench.ps1 `
#     [-Exe target\release\wallive.exe] [-WalliveArgs '--play "C:\v.mp4"'] `
#     [-WarmupSec 10] [-Seconds 60] [-Label name] [-Minimize] [-Attach <pid>]
#
# Reports:
#   cpu1   % of one logical core (process CPU time / wall time)
#   cpuAll % of the whole CPU (cpu1 / logical cores) - the budget number
#   ws / priv  working set and private bytes at the end (MB)
#   gpu    average utilisation per GPU engine type for this process (%)
#   batt   battery discharge rate (mW) when on battery, else "AC"
# -Attach measures an already-running process instead of starting one.
param(
  [string]$Exe = '',
  [string]$WalliveArgs = '',
  [int]$WarmupSec = 10,
  [int]$Seconds = 60,
  [string]$Label = '',
  [switch]$Minimize,
  [int]$Attach = 0,
  [switch]$KeepRunning
)
$ErrorActionPreference = 'Stop'
# $PSScriptRoot is empty in param defaults on Windows PowerShell 5.1.
if (-not $Exe) { $Exe = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\target\release\wallive.exe' }
Add-Type -Name Win -Namespace Bench -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
'@

$started = $false
if ($Attach -ne 0) {
  $proc = Get-Process -Id $Attach
} else {
  $log = Join-Path $env:TEMP ("wallive-bench-{0}.log" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
  $start = @{ FilePath = $Exe; PassThru = $true; WindowStyle = 'Hidden'; RedirectStandardError = $log }
  if ($WalliveArgs) { $start.ArgumentList = $WalliveArgs }
  $proc = Start-Process @start
  $started = $true
}
$shell = $null
if ($Minimize) { $shell = New-Object -ComObject Shell.Application; $shell.MinimizeAll() }
Start-Sleep -Seconds $WarmupSec
if ($proc.HasExited) { throw "wallive exited early (code $($proc.ExitCode)); log: $log" }

$proc.Refresh(); $cpu0 = $proc.TotalProcessorTime.TotalMilliseconds; $t0 = [Diagnostics.Stopwatch]::StartNew()
$interval = 2
$samples = [math]::Max(1, [int]($Seconds / $interval))
$counter = "\GPU Engine(pid_$($proc.Id)_*)\Utilization Percentage"
$gpu = @{}
$batt = New-Object System.Collections.Generic.List[double]
for ($i = 0; $i -lt $samples; $i++) {
  $due = ($i + 1) * $interval * 1000
  try {
    $s = Get-Counter -Counter $counter -SampleInterval $interval -MaxSamples 1 -ErrorAction Stop
    foreach ($c in $s.CounterSamples) {
      $type = ($c.InstanceName -split 'engtype_')[-1]
      $gpu[$type] = [double]$gpu[$type] + $c.CookedValue
    }
  } catch {}   # no GPU engine instances for this process
  try {
    $b = Get-CimInstance -Namespace root\wmi -ClassName BatteryStatus -ErrorAction Stop | Select-Object -First 1
    if ($b -and -not $b.PowerOnline -and $b.DischargeRate -gt 0) { $batt.Add($b.DischargeRate) }
  } catch {}
  # Keep the window time-based even when a counter read fails fast.
  $left = $due - $t0.Elapsed.TotalMilliseconds
  if ($left -gt 0) { Start-Sleep -Milliseconds ([int]$left) }
}
$proc.Refresh()
$elapsed = $t0.Elapsed.TotalMilliseconds
$cpu1 = 100.0 * ($proc.TotalProcessorTime.TotalMilliseconds - $cpu0) / $elapsed
$cores = [Environment]::ProcessorCount
$ws = $proc.WorkingSet64 / 1MB; $priv = $proc.PrivateMemorySize64 / 1MB; $threads = $proc.Threads.Count
$gpuText = ($gpu.GetEnumerator() | Where-Object { $_.Value / $samples -ge 0.05 } | Sort-Object Name |
  ForEach-Object { '{0}={1:N1}' -f $_.Name, ($_.Value / $samples) }) -join ' '
if (-not $gpuText) { $gpuText = 'all<0.05' }
$battText = if ($batt.Count) { '{0:N0}mW' -f ($batt | Measure-Object -Average).Average } else { 'AC' }

if ($Minimize -and $shell) { $shell.UndoMinimizeALL() }
if ($started -and -not $KeepRunning) {
  $h = [Bench.Win]::FindWindowW('WalliveHost', [NullString]::Value)
  if ($h -ne [IntPtr]::Zero) { [Bench.Win]::PostMessageW($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null }
  if (-not $proc.WaitForExit(5000)) { Stop-Process -Id $proc.Id -Force; Write-Warning 'had to kill wallive' }
}

'{0} | {1}s | cpu1 {2:N2}% | cpuAll {3:N3}% | ws {4:N1} MB | priv {5:N1} MB | threads {6} | gpu {7} | batt {8}' -f `
  $Label, [int]($elapsed / 1000), $cpu1, ($cpu1 / $cores), $ws, $priv, $threads, $gpuText, $battText
if ($started) { "log: $log" }
