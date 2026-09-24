; The bettercut installer, built by docs\package.ps1 with Inno Setup 6.
;
; package.ps1 stages the files and passes AppVersion, StageDir and OutputDir;
; this script only says how they are installed. A per-user install by
; default, so a tester needs no administrator rights; an uninstaller is added
; to Settings > Apps, and .vproj files open in bettercut.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef StageDir
  #error StageDir not given - run docs\package.ps1 rather than this script
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{6E0B7C2A-4F1D-4B8E-9C3A-B5E7D2F14A90}
AppName=bettercut
AppVersion={#AppVersion}
AppPublisher=bettercut
DefaultDirName={autopf}\bettercut
DefaultGroupName=bettercut
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=bettercut-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\bettercut.exe
ChangesAssociations=yes

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\bettercut"; Filename: "{app}\bettercut.exe"
Name: "{autodesktop}\bettercut"; Filename: "{app}\bettercut.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Registry]
; .vproj files open in bettercut: the path arrives as the first argument,
; which main.rs already opens.
Root: HKA; Subkey: "Software\Classes\.vproj"; ValueType: string; ValueData: "bettercut.Project"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\bettercut.Project"; ValueType: string; ValueData: "bettercut project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\bettercut.Project\shell\open\command"; ValueType: string; ValueData: """{app}\bettercut.exe"" ""%1"""

[Run]
Filename: "{app}\bettercut.exe"; Description: "Start bettercut"; Flags: nowait postinstall skipifsilent
