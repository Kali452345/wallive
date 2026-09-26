# Pause-policy check on the real desktop (ADR-005). Starts wallive with a
# video, then drives the desktop and reads wallive's log to see that it
# pauses and resumes for the right reasons, and measures CPU in each phase.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/pause-check.ps1 -Video C:\v.mp4 `
#     [-Exe target\release\wallive.exe] [-PhaseSec 10] [-SkipDisplayOff]
#
# Phases: playing -> maximized window (covered, paused) -> cursor storm
# while paused (hook cost) -> closed (playing) -> half-screen window (still
# playing) -> display off (paused) -> display on (playing).
# The display-off phase turns the monitor off with SC_MONITORPOWER and wakes
# it with a synthetic mouse move; skip it on Modern Standby machines, where
# display off can put the system to sleep.
param(
  [Parameter(Mandatory = $true)][string]$Video,
  [string]$Exe = '',
  [int]$PhaseSec = 10,
  [switch]$SkipDisplayOff
)
$ErrorActionPreference = 'Stop'
# $PSScriptRoot is empty in param defaults on Windows PowerShell 5.1.
if (-not $Exe) { $Exe = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\target\release\wallive.exe' }
Add-Type -Name Win -Namespace PauseCheck -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
[DllImport("user32.dll")] public static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint ms, out IntPtr result);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, IntPtr extra);
'@

$log = Join-Path $env:TEMP ("wallive-pause-{0}.log" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
$proc = Start-Process -FilePath $Exe -ArgumentList "--play `"$Video`"" -PassThru -WindowStyle Hidden -RedirectStandardError $log
$cores = [Environment]::ProcessorCount
$results = New-Object System.Collections.Generic.List[string]

function Measure-Phase([string]$name, [int]$sec) {
  $proc.Refresh(); $c0 = $proc.TotalProcessorTime.TotalMilliseconds; $t = [Diagnostics.Stopwatch]::StartNew()
  Start-Sleep -Seconds $sec
  $proc.Refresh(); $cpu1 = 100.0 * ($proc.TotalProcessorTime.TotalMilliseconds - $c0) / $t.Elapsed.TotalMilliseconds
  $line = '{0,-22} cpu1 {1,6:N2}%  cpuAll {2,6:N3}%  ws {3,6:N1} MB' -f $name, $cpu1, ($cpu1 / $cores), ($proc.WorkingSet64 / 1MB)
  $results.Add($line); $line
}

# A plain window in another process; -State Maximized or a fixed rectangle.
function Start-Window([string]$state, [int]$w = 0, [int]$h = 0) {
  $code = "Add-Type -AssemblyName System.Windows.Forms; `$f = New-Object Windows.Forms.Form; `$f.Text = 'pause-check'; "
  if ($state -eq 'Maximized') { $code += "`$f.WindowState = 'Maximized'; " }
  else { $code += "`$f.StartPosition = 'Manual'; `$f.Location = New-Object Drawing.Point(0, 0); `$f.Size = New-Object Drawing.Size($w, $h); " }
  $code += "`$f.TopMost = `$true; [void]`$f.ShowDialog()"
  Start-Process powershell -ArgumentList '-NoProfile', '-Command', $code -PassThru
}

function Log-Since([int]$from) { (Get-Content $log)[$from..10000] -join "`n" }
function Log-Count { @(Get-Content $log).Count }

try {
  Start-Sleep -Seconds 8
  if ($proc.HasExited) { throw "wallive exited early; log: $log" }
  Measure-Phase 'playing' $PhaseSec

  $mark = Log-Count
  $win = Start-Window 'Maximized'
  Start-Sleep -Seconds 3
  Measure-Phase 'maximized window' $PhaseSec
  $results.Add("  log: " + ((Log-Since $mark) -split "`n" | Where-Object { $_ -match 'pause|resumed' }) -join ' / ')

  # Cursor storm while paused, so the hook cost is not hidden by playback:
  # every move is an EVENT_OBJECT_LOCATIONCHANGE for the cursor.
  $storm = Start-Job -ScriptBlock {
    Add-Type -Name W -Namespace S -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, IntPtr extra);'
    $sw = [Diagnostics.Stopwatch]::StartNew(); $i = 0
    # ~1000 relative moves/s through the input stack, like a high-rate
    # gaming mouse (SetCursorPos does not raise the same events). Spin
    # because Start-Sleep cannot wait 1 ms.
    while ($sw.Elapsed.TotalSeconds -lt $using:PhaseSec) {
      [S.W]::mouse_event(1, $(if ($i % 2) { 3 } else { -3 }), 0, 0, [IntPtr]::Zero); $i++
      while ($sw.Elapsed.TotalMilliseconds -lt $i) {}
    }
    $i
  }
  Measure-Phase 'covered + cursor storm' $PhaseSec
  $moves = Receive-Job $storm -Wait; Remove-Job $storm
  $results.Add("  cursor moves: $moves")

  $mark = Log-Count
  Stop-Process -Id $win.Id -Force
  Start-Sleep -Seconds 2
  Measure-Phase 'window closed' $PhaseSec
  $results.Add("  log: " + ((Log-Since $mark) -split "`n" | Where-Object { $_ -match 'pause|resumed' }) -join ' / ')

  $mark = Log-Count
  $win = Start-Window 'Normal' 960 1000
  Start-Sleep -Seconds 3
  Measure-Phase 'half-screen window' $PhaseSec
  $results.Add("  log: " + ((Log-Since $mark) -split "`n" | Where-Object { $_ -match 'pause|resumed' }) -join ' / ')
  Stop-Process -Id $win.Id -Force
  Start-Sleep -Seconds 2

  if (-not $SkipDisplayOff) {
    $mark = Log-Count
    $out = [IntPtr]::Zero
    # WM_SYSCOMMAND / SC_MONITORPOWER / 2 = off, to the broadcast window.
    [void][PauseCheck.Win]::SendMessageTimeoutW([IntPtr]0xffff, 0x0112, [IntPtr]0xF170, [IntPtr]2, 2, 2000, [ref]$out)
    Start-Sleep -Seconds 4
    Measure-Phase 'display off' $PhaseSec
    $results.Add("  log: " + ((Log-Since $mark) -split "`n" | Where-Object { $_ -match 'pause|resumed|power' }) -join ' / ')
    $mark = Log-Count
    [PauseCheck.Win]::mouse_event(1, 10, 0, 0, [IntPtr]::Zero); [PauseCheck.Win]::mouse_event(1, -10, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Seconds 4
    Measure-Phase 'display on' $PhaseSec
    $results.Add("  log: " + ((Log-Since $mark) -split "`n" | Where-Object { $_ -match 'pause|resumed|power' }) -join ' / ')
  }
} finally {
  $h = [PauseCheck.Win]::FindWindowW('WalliveHost', [NullString]::Value)
  if ($h -ne [IntPtr]::Zero) { [void][PauseCheck.Win]::PostMessageW($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
  if (-not $proc.WaitForExit(5000)) { Stop-Process -Id $proc.Id -Force; Write-Warning 'had to kill wallive' }
}
''
'=== summary ==='
$results
"log: $log"
