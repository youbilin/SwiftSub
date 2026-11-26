#ifndef MyAppVersion
  #define MyAppVersion "1.1.0"
#endif

[Setup]
AppId={{7D2D6E48-4C46-4F1D-A6C2-58B3B8D8C101}
AppName=SwiftSub
AppVersion={#MyAppVersion}
AppVerName=SwiftSub v{#MyAppVersion}
AppPublisher=SwiftSub
AppPublisherURL=https://github.com/youbilin/SwiftSub
AppSupportURL=https://github.com/youbilin/SwiftSub/issues
AppUpdatesURL=https://github.com/youbilin/SwiftSub/releases
DefaultDirName={autopf}\SwiftSub
DefaultGroupName=SwiftSub
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\swiftsub.ico
UninstallDisplayIcon={app}\swiftsub.exe
UninstallDisplayName=SwiftSub
CloseApplications=force
RestartApplications=no
OutputDir=..\dist
OutputBaseFilename=SwiftSub-v{#MyAppVersion}-windows-x64-Setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "Start SwiftSub silently in system tray on Windows login"; GroupDescription: "Startup:"; Flags: unchecked

[Files]
Source: "..\target\release\swiftsub.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "installed.marker"; DestDir: "{app}"; DestName: ".installed"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\SwiftSub"; Filename: "{app}\swiftsub.exe"
Name: "{autodesktop}\SwiftSub"; Filename: "{app}\swiftsub.exe"; Tasks: desktopicon
Name: "{autoprograms}\Uninstall SwiftSub"; Filename: "{uninstallexe}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "SwiftSub"; ValueData: """{app}\swiftsub.exe"" --background"; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\swiftsub.exe"; Description: "{cm:LaunchProgram,SwiftSub}"; Flags: nowait postinstall skipifsilent

[Code]
procedure StopRunningSwiftSub();
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM swiftsub.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningSwiftSub();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopRunningSwiftSub();
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'SwiftSub');
  end;
end;
