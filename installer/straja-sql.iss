#ifndef AppVersion
  #define AppVersion "0.1.13"
#endif
#ifdef InstallerTest
  #define ProductName "StrajaBackupSQL Installer Test"
  #define ProductId "{{34CA39E7-0EBA-42C2-9042-AD11A00DF38C}"
  #define FolderName "StrajaBackupSQLInstallerTest"
  #define Suffix "-test"
#else
  #define ProductName "StrajaBackupSQL"
  #define ProductId "{{068559EE-3762-4C65-8A38-3935B577810F}"
  #define FolderName "StrajaBackupSQL"
  #define Suffix ""
#endif
[Setup]
AppId={#ProductId}
AppName={#ProductName}
AppVersion={#AppVersion}
AppPublisher=StrajaBackupSQL
AppComments=Database Backup Studio for MariaDB, MySQL and PostgreSQL
AppPublisherURL=https://strajabackup.com
LanguageDetectionMethod=none
ShowLanguageDialog=yes
DefaultDirName={localappdata}\Programs\{#FolderName}
DefaultGroupName={#ProductName}
DisableProgramGroupPage=yes
DisableWelcomePage=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist\installer
OutputBaseFilename=StrajaBackupSQL-{#AppVersion}-windows-x64{#Suffix}-setup
Compression=lzma2/ultra64
SolidCompression=yes
SetupIconFile=assets\straja-sql.ico
UninstallDisplayIcon={app}\straja-sql.ico
UninstallDisplayName={#ProductName}
VersionInfoVersion={#AppVersion}.0
VersionInfoDescription=StrajaBackupSQL · Database Backup Studio
WizardStyle=modern light windows11 includetitlebar
WizardSizePercent=120,115
WizardImageFile=assets\wizard.png
WizardSmallImageFile=assets\logo.png
WizardImageBackColor=$301D10
InfoBeforeFile=prerequisites.en.txt
SetupLogging=yes
CloseApplications=no
RestartApplications=no
#ifdef InstallerTest
; A separate installation of the new binaries does not replace the running 0.1.0 binaries.
AppMutex=Local\StrajaBackupSQL.Window,Local\StrajaBackupSQL.Agent
#else
AppMutex=Local\StrajaBackupSQL.Window,Local\StrajaBackupSQL.Agent,Local\StrajaSQLWindow,Local\StrajaSQLAgent
#endif
[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"; InfoBeforeFile: "prerequisites.en.txt"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"; InfoBeforeFile: "prerequisites.de.txt"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"; InfoBeforeFile: "prerequisites.es.txt"
Name: "pt"; MessagesFile: "compiler:Languages\Portuguese.isl"; InfoBeforeFile: "prerequisites.pt.txt"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"; InfoBeforeFile: "prerequisites.fr.txt"
Name: "it"; MessagesFile: "compiler:Languages\Italian.isl"; InfoBeforeFile: "prerequisites.it.txt"
[LangOptions]
DialogFontName=Segoe UI
DialogFontSize=10
WelcomeFontName=Segoe UI
WelcomeFontSize=20
#include "localized-messages.iss"
[Tasks]
Name: "desktop"; Description: "{cm:Desktop}"; Flags: unchecked
Name: "agentlogin"; Description: "{cm:AgentLogin}"
[Files]
Source: "..\dist\StrajaBackupSQL\StrajaBackupSQL.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\StrajaBackupSQL\StrajaBackupSQL-Agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\StrajaBackupSQL\*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\StrajaBackupSQL\tools\*"; DestDir: "{app}\tools"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "assets\straja-sql.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\SQL-TOOLS-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "language.en.txt"; DestDir: "{app}"; DestName: "default-language.txt"; Languages: en; Flags: ignoreversion
Source: "language.de.txt"; DestDir: "{app}"; DestName: "default-language.txt"; Languages: de; Flags: ignoreversion
Source: "language.es.txt"; DestDir: "{app}"; DestName: "default-language.txt"; Languages: es; Flags: ignoreversion
Source: "language.pt.txt"; DestDir: "{app}"; DestName: "default-language.txt"; Languages: pt; Flags: ignoreversion
Source: "language.fr.txt"; DestDir: "{app}"; DestName: "default-language.txt"; Languages: fr; Flags: ignoreversion
Source: "language.it.txt"; DestDir: "{app}"; DestName: "default-language.txt"; Languages: it; Flags: ignoreversion
Source: "..\docs\*.txt"; DestDir: "{app}\docs"; Flags: ignoreversion
[Icons]
Name: "{group}\{#ProductName}"; Filename: "{app}\StrajaBackupSQL.exe"; IconFilename: "{app}\straja-sql.ico"
Name: "{autodesktop}\{#ProductName}"; Filename: "{app}\StrajaBackupSQL.exe"; Tasks: desktop; IconFilename: "{app}\straja-sql.ico"
Name: "{group}\{cm:Uninstall}"; Filename: "{uninstallexe}"
[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#FolderName}.Agent"; ValueData: """{app}\StrajaBackupSQL-Agent.exe"""; Tasks: agentlogin; Flags: uninsdeletevalue
[Run]
Filename: "{app}\StrajaBackupSQL-Agent.exe"; Flags: nowait skipifsilent; Tasks: agentlogin
Filename: "{app}\StrajaBackupSQL.exe"; Description: "{cm:OpenApp}"; Flags: nowait postinstall skipifsilent
[Code]
function InitializeUninstall(): Boolean;
begin
#ifdef InstallerTest
  Result := not (CheckForMutexes('Local\StrajaBackupSQL.Window') or CheckForMutexes('Local\StrajaBackupSQL.Agent'));
#else
  Result := not (CheckForMutexes('Local\StrajaBackupSQL.Window') or CheckForMutexes('Local\StrajaBackupSQL.Agent') or CheckForMutexes('Local\StrajaSQLWindow') or CheckForMutexes('Local\StrajaSQLAgent'));
#endif
  if not Result then MsgBox(CustomMessage('CloseFirst'), mbInformation, MB_OK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  OldCommand, ExpectedCommand: String;
begin
  if CurStep = ssPostInstall then begin
    ExpectedCommand := '"' + ExpandConstant('{app}\straja-sql.exe') + '" --agent';
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'StrajaSQLAgent', OldCommand) then
      if CompareText(OldCommand, ExpectedCommand) = 0 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'StrajaSQLAgent');
  end;
end;

[InstallDelete]
Type: files; Name: "{app}\straja-sql.exe"
