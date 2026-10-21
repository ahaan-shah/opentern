; Opentern installer (Inno Setup 6.3+), adapted from Magpie's. Built by
; scripts/package-windows.ps1:
;   iscc /DVersion=0.1.2 /DSrcX64=...\opentern.exe /DSrcArm64=...\opentern.exe opentern.iss
;
; One installer for x64 and ARM64 PCs: it installs the native build for the
; machine. Installs per user by default (no admin prompt) into
; %LOCALAPPDATA%\Programs\Opentern; "Install for all users" is offered too.
; Upgrades install in place (same AppId) and close a running Opentern first.
; Uninstalling leaves your workspace (in %APPDATA%\opentern) alone.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef SrcX64
  #define SrcX64 "..\..\target\x86_64-pc-windows-msvc\release\opentern.exe"
#endif
#ifndef SrcArm64
  #define SrcArm64 "..\..\target\aarch64-pc-windows-msvc\release\opentern.exe"
#endif
#ifndef OutDir
  #define OutDir "..\..\dist"
#endif

[Setup]
; Never change AppId: it's how Windows recognises an upgrade.
AppId={{6B43380A-7DA8-43A5-98A1-57609724C945}
AppName=Opentern
AppVersion={#Version}
AppVerName=Opentern {#Version}
AppPublisher=Ahaan Shah
AppPublisherURL=https://github.com/ahaan-shah/opentern
AppSupportURL=https://github.com/ahaan-shah/opentern/issues
AppUpdatesURL=https://github.com/ahaan-shah/opentern/releases
VersionInfoVersion={#Version}
VersionInfoProductName=Opentern
DefaultDirName={autopf}\Opentern
DefaultGroupName=Opentern
DisableProgramGroupPage=yes
DisableDirPage=auto
DisableReadyPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
ArchitecturesAllowed=x64compatible or arm64
ArchitecturesInstallIn64BitMode=x64compatible or arm64
MinVersion=10.0.17763
OutputDir={#OutDir}
OutputBaseFilename=Opentern-windows-setup
SetupIconFile=..\..\crates\app\assets\opentern.ico
UninstallDisplayIcon={app}\opentern.exe
UninstallDisplayName=Opentern
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=force
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SrcX64}"; DestDir: "{app}"; DestName: "opentern.exe"; Check: not IsArm64; Flags: ignoreversion
Source: "{#SrcArm64}"; DestDir: "{app}"; DestName: "opentern.exe"; Check: IsArm64; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Opentern"; Filename: "{app}\opentern.exe"; Comment: "Market analysis terminal"
Name: "{autodesktop}\Opentern"; Filename: "{app}\opentern.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\opentern.exe"; Description: "{cm:LaunchProgram,Opentern}"; Flags: nowait postinstall skipifsilent
