; Compile with Inno Setup 6 after staging target/package/windows.
#ifndef AppVersion
  #error AppVersion must be supplied
#endif
#ifndef NativeVersion
  #error NativeVersion must be supplied
#endif

[Setup]
AppId={{B3F4C53F-77C7-4639-9938-C01A21F4A0B9}
AppName=Evanya
AppVersion={#AppVersion}
VersionInfoVersion={#NativeVersion}
AppPublisher=Flectar
AppPublisherURL=https://flectar.com
DefaultDirName={localappdata}\Programs\Evanya
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\evanya.exe
SetupIconFile=..\..\resources\app-icon\evanya.ico
OutputDir=..\..\target
OutputBaseFilename=evanya-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Files]
Source: "..\..\target\package\windows\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\Evanya"; Filename: "{app}\evanya.exe"

[Run]
Filename: "{app}\evanya.exe"; Description: "Open Evanya"; Flags: nowait postinstall skipifsilent

[Code]
// The app registers its own per-user sign-in entry; remove it with the app so
// Windows does not keep a startup item that points at a deleted executable.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Evanya');
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run', 'Evanya');
  end;
end;
