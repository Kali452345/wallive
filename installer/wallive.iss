; Inno Setup script for Wallive (ADR-013). Build with tools/package.ps1,
; which passes the version from Cargo.toml:
;   ISCC.exe /DAppVersion=1.0.0 installer\wallive.iss
; Per-user install, no admin rights: %LOCALAPPDATA%\Programs\Wallive.

#ifndef AppVersion
  #error Pass the version: ISCC /DAppVersion=x.y.z installer\wallive.iss
#endif

#define AppName "Wallive"
#define AppExe "wallive.exe"
#define AppUrl "https://github.com/Kali452345/wallive"

[Setup]
; Never change AppId: it links upgrades and the uninstall entry.
AppId={{DFF10C26-3731-4421-88A0-5662CB36DD83}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Kali452345
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppComments=Live video wallpaper with hardware decoding
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\LICENSE
SetupIconFile=wallive.ico
UninstallDisplayIcon={app}\wallive.ico
UninstallDisplayName={#AppName}
OutputDir=..\dist
OutputBaseFilename=Wallive-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The running copy is closed through its window (see [Code]); the Restart
; Manager cannot close a tray app without a main window.
CloseApplications=no
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start Wallive when I sign in to Windows"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "wallive.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; IconFilename: "{app}\wallive.ico"; Comment: "Live video wallpaper"

[Registry]
; Same value the tray menu's "Start with Windows" writes (src/shell/ffi.rs).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Wallive"; ValueData: """{app}\{#AppExe}"""; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "Start Wallive now"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Imported videos, log and settings.
Type: filesandordirs; Name: "{localappdata}\Wallive"
Type: filesandordirs; Name: "{userappdata}\Wallive"

[Code]
const
  WM_CLOSE = $0010;
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

{ Asks a running Wallive to exit (as `wallive --quit` does) and waits up to
  10 s for its window to go away, so its files can be replaced or removed. }
procedure CloseRunningWallive;
var
  Wnd: HWND;
  Waited: Integer;
begin
  Wnd := FindWindowByClassName('WalliveHost');
  if Wnd = 0 then
    Exit;
  PostMessage(Wnd, WM_CLOSE, 0, 0);
  Waited := 0;
  while (FindWindowByClassName('WalliveHost') <> 0) and (Waited < 10000) do
  begin
    Sleep(100);
    Waited := Waited + 100;
  end;
  { The process ends just after its window; give it a moment. }
  Sleep(300);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningWallive;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Value: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    CloseRunningWallive;
    { Autostart may also have been turned on from the tray menu; remove it
      if it points at this installation. }
    if RegQueryStringValue(HKCU, RunKey, 'Wallive', Value) and
       (Pos(Lowercase(ExpandConstant('{app}')), Lowercase(Value)) > 0) then
      RegDeleteValue(HKCU, RunKey, 'Wallive');
  end;
end;
