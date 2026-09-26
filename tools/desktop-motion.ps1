# Minimises all windows, takes two screenshots of the primary screen a moment
# apart, restores the windows, and reports how many sampled pixels changed.
# Proves the wallpaper is animating without a human looking at it.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File tools/desktop-motion.ps1 -Out <dir> [-GapMs 700] [-KeepMinimized]
param([Parameter(Mandatory)][string]$Out, [int]$GapMs = 700, [switch]$KeepMinimized)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Name Dpi -Namespace M -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[M.Dpi]::SetProcessDPIAware() | Out-Null
$shell = New-Object -ComObject Shell.Application
$shell.MinimizeAll(); Start-Sleep -Milliseconds 1500
function Grab([string]$path) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size); $g.Dispose()
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); return $bmp
}
$a = Grab (Join-Path $Out 'motion-a.png'); Start-Sleep -Milliseconds $GapMs; $b = Grab (Join-Path $Out 'motion-b.png')
if (-not $KeepMinimized) { $shell.UndoMinimizeALL() }
$changed = 0; $total = 0
for ($y = 0; $y -lt $a.Height; $y += 20) { for ($x = 0; $x -lt $a.Width; $x += 20) {
  $p = $a.GetPixel($x, $y); $q = $b.GetPixel($x, $y); $total++
  if ([math]::Abs($p.R - $q.R) + [math]::Abs($p.G - $q.G) + [math]::Abs($p.B - $q.B) -gt 24) { $changed++ } } }
"sampled=$total changed=$changed ({0:N1}%)" -f (100.0 * $changed / $total)
