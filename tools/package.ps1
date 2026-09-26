# Builds the release exe and the Inno Setup installer (ADR-013).
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools/package.ps1 [-NoBuild] [-Iscc <path>]
#
# Output: dist\Wallive-<version>-setup.exe and a .sha256 file next to it.
# The version comes from Cargo.toml. Needs Inno Setup 6.3 or later
# (https://jrsoftware.org/isinfo.php); -Iscc or the ISCC environment variable
# overrides where ISCC.exe is looked for.
param([switch]$NoBuild, [string]$Iscc = $env:ISCC)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw 'version not found in Cargo.toml' }

if (-not $NoBuild) {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
$exe = Join-Path $root 'target\release\wallive.exe'
if (-not (Test-Path $exe)) { throw "missing $exe" }
# The exe must be the version being packaged (a stale -NoBuild exe is not).
$reported = (& $exe --version | Out-String).Trim()
if ($reported -ne "wallive $version") { throw "exe reports '$reported', Cargo.toml says $version" }

if (-not $Iscc) {
    $candidates = @(
        (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    )
    $Iscc = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $Iscc) { throw 'ISCC.exe not found; install Inno Setup 6 or pass -Iscc' }

& $Iscc /Q "/DAppVersion=$version" installer\wallive.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }

$setup = Join-Path $root "dist\Wallive-$version-setup.exe"
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
"$hash  Wallive-$version-setup.exe" | Set-Content "$setup.sha256" -Encoding ascii
"{0}  {1:N1} MB  sha256 {2}" -f $setup, ((Get-Item $setup).Length / 1MB), $hash
