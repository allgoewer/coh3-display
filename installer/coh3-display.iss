; Inno Setup script for the CoH3 Display tray app. Build with installer\build.ps1, which
; passes the version from Cargo.toml as /DAppVersion=x.y.z.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef AppNumericVersion
  #define AppNumericVersion AppVersion
#endif
#define AppName "CoH3 Display"
#define AppExe "coh3-display.exe"

[Setup]
; Identifies the app across versions, so a newer setup upgrades in place. Never change it.
AppId={{B6E0C9E4-6C7B-4B5E-9C3B-5B0B7A4D2F61}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=CoH3 Display
VersionInfoVersion={#AppNumericVersion}
; Per-user install: no administrator rights, no UAC prompt.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
; The tray app holds this mutex (src/win.rs); setup and uninstall ask to quit it first.
AppMutex=Local\coh3-display-tracker
SetupIconFile=coh3-display.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
OutputDir=..\target\installer
OutputBaseFilename=coh3-display-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start {#AppName} when I sign in to Windows"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"

[Registry]
; The same value the tray menu's "Start with Windows" item manages (src/autostart.rs).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "coh3-display"; ValueData: """{app}\{#AppExe}"""; Tasks: autostart
; Whoever set it, it must not outlive the program.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "coh3-display"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
// Settings and the log are kept by default, so a reinstall picks up where it left off.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
  begin
    if MsgBox('Also delete your CoH3 Display settings and log file?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\coh3-display'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\coh3-display'), True, True, True);
    end;
  end;
end;
