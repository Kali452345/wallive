# Dumps Explorer's desktop window tree (Progman children in z-order, top-level
# WorkerW windows) and optionally saves a half-size screenshot of the virtual
# screen. Diagnostic only; never part of the running app.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File tools/inspect-desktop.ps1 [-Shot out.png]
# Note: pass [NullString]::Value, not $null, for P/Invoke string args - PowerShell turns $null into "".
param([string]$Shot)
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr a, string c, string n);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public struct RECT { public int L, T, R, B; }
  public static string Cls(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
}
"@
[W]::SetProcessDPIAware() | Out-Null
function Show($h, $indent) {
  $r = New-Object W+RECT; [W]::GetWindowRect($h, [ref]$r) | Out-Null
  $pid_ = 0; [W]::GetWindowThreadProcessId($h, [ref]$pid_) | Out-Null
  $ex = [W]::GetWindowLong($h, -20)
  "{0}{1,-18} 0x{2:X8} vis={3} ex=0x{4:X8} pid={5} rect=({6},{7})-({8},{9})" -f $indent, [W]::Cls($h), $h.ToInt64(), [W]::IsWindowVisible($h), $ex, $pid_, $r.L, $r.T, $r.R, $r.B
}
$progman = [W]::FindWindow("Progman", [NullString]::Value)
"Progman:"; Show $progman "  "
"Progman children (topmost first):"
$c = [W]::GetWindow($progman, 5)
while ($c -ne [IntPtr]::Zero) { Show $c "  "; $g = [W]::GetWindow($c, 5); while ($g -ne [IntPtr]::Zero) { Show $g "      "; $g = [W]::GetWindow($g, 2) }; $c = [W]::GetWindow($c, 2) }
"Top-level WorkerW:"
$a = [IntPtr]::Zero
while (($a = [W]::FindWindowEx([IntPtr]::Zero, $a, "WorkerW", [NullString]::Value)) -ne [IntPtr]::Zero) { Show $a "  " }
if ($Shot) {
  Add-Type -AssemblyName System.Windows.Forms, System.Drawing
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
  $small = New-Object System.Drawing.Bitmap $bmp, ([int]($b.Width/2)), ([int]($b.Height/2))
  $small.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png); "screenshot: $Shot ($($b.Width)x$($b.Height))"
}

