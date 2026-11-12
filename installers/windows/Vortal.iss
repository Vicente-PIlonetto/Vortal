#define AppName "Vortal"
#define AppVersion "0.1.0"
#define AppPublisher "Vortal"
#define RepoRoot AddBackslash(SourcePath) + "..\.."

[Setup]
AppId={{6A0B5C31-AC7B-4B83-982B-5D18C95D03A8}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={localappdata}\Programs\Vortal
DefaultGroupName=Vortal
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputDir={#RepoRoot}\dist\windows
OutputBaseFilename=Vortal-Setup-x64
SetupIconFile={#RepoRoot}\crates\vortal-desktop\assets\vortal.ico
UninstallDisplayIcon={app}\vortal.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
VersionInfoVersion=0.1.0.0
VersionInfoProductName=Vortal
VersionInfoDescription=Vortal desktop test build

[Tasks]
Name: "desktopicon"; Description: "Criar atalho na área de trabalho"; GroupDescription: "Atalhos:"; Flags: unchecked

[Files]
Source: "{#RepoRoot}\target\release\vortal-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\target\release\vortal.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\target\release\vortald.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\crates\vortal-desktop\assets\vortal.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\installers\windows\README.md"; DestDir: "{app}"; DestName: "README-WINDOWS.md"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Vortal"; Filename: "{app}\vortal-desktop.exe"; IconFilename: "{app}\vortal.ico"
Name: "{autodesktop}\Vortal"; Filename: "{app}\vortal-desktop.exe"; IconFilename: "{app}\vortal.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\vortal-desktop.exe"; Description: "Abrir Vortal"; Flags: postinstall nowait skipifsilent
