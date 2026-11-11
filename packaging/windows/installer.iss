; Inno Setup 6.3+ script (https://jrsoftware.org/isinfo.php). On Windows, from the project folder:
;   cargo build --release --locked --target x86_64-pc-windows-msvc
;   iscc packaging\windows\installer.iss      -> dist\House-Energy-Tracker-Setup-<version>.exe
; The version comes from the exe (that is, from Cargo.toml). To sign the installer and its uninstaller,
; see "Signing" in README.md.
; Installs for the current user only (no admin prompt). Your data is kept on uninstall.
#define AppName "House Energy Tracker"
; Only the MSVC build: a GNU build needs WebView2Loader.dll beside it and wouldn't start.
#ifndef AppExe
  #define AppExe AddBackslash(SourcePath) + "..\..\target\x86_64-pc-windows-msvc\release\energy-tracker.exe"
#endif
#if !FileExists(AppExe)
  #error Build the exe first: cargo build --release --locked --target x86_64-pc-windows-msvc
#endif
#ifndef AppVersion
  #define AppVersion GetStringFileInfo(AppExe, "ProductVersion")
#endif

[Setup]
AppId={{6C1F3B0E-6E52-4B5B-9E1A-2F4E3A9D7C21}
AppName={#AppName}
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=House-Energy-Tracker-Setup-{#AppVersion}
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\energy-tracker.exe
UninstallDisplayName={#AppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
#ifdef Sign
SignTool=signtool
SignedUninstaller=yes
#endif

[Files]
#ifdef Sign
Source: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion signonce
#else
Source: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
#endif

[Icons]
Name: "{userprograms}\{#AppName}"; Filename: "{app}\energy-tracker.exe"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\energy-tracker.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\energy-tracker.exe"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Only if asking it to quit (below) didn't work.
Filename: "taskkill.exe"; Parameters: "/F /IM energy-tracker.exe"; Flags: runhidden; RunOnceId: "StopApp"

[UninstallDelete]
; The weather cache, log and the window's own storage. Your data (in %APPDATA%) is kept.
Type: filesandordirs; Name: "{localappdata}\{#AppName}"

[Code]
// The window needs the Microsoft Edge WebView2 Runtime. It comes with Windows 11 and current Windows 10;
// without it the app opens in the browser instead, so this only warns.
function HasWebView2(): Boolean;
var v: String;
begin
  Result :=
    RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', 'pv', v) or
    RegQueryStringValue(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', 'pv', v);
  Result := Result and (v <> '') and (v <> '0.0.0.0');
end;

procedure CurStepChanged(CurStep: TSetupStep);
var code: Integer;
begin
  if (CurStep = ssPostInstall) and not HasWebView2() and not WizardSilent() then
    if MsgBox('House Energy Tracker uses the Microsoft Edge WebView2 Runtime for its window, and it isn''t installed. ' +
              'Download it now? (Without it, the app opens in your web browser instead.)', mbConfirmation, MB_YESNO) = IDYES then
      // Microsoft's small "Evergreen Bootstrapper" installer.
      ShellExec('open', 'https://go.microsoft.com/fwlink/p/?LinkId=2124703', '', '', SW_SHOWNORMAL, ewNoWait, code);
end;

// Ask a running copy to save and close before its files are removed (taskkill above is the fallback).
function InitializeUninstall(): Boolean;
var http: Variant;
begin
  try
    http := CreateOleObject('WinHttp.WinHttpRequest.5.1');
    http.SetTimeouts(1000, 1000, 1000, 1000);
    http.Open('POST', 'http://127.0.0.1:47615/api/quit', False);
    http.Send('');
    Sleep(2000);
  except
  end;
  Result := True;
end;
