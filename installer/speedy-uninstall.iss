; ============================================================
;  Speedy — Uninstaller runner
;
;  Installs nothing. When run, it looks for Speedy's real uninstaller
;  in %LOCALAPPDATA%\Programs\Speedy\ and launches it.
;
;  Build:
;    iscc /DMyAppVersion=0.1.0 installer\speedy-uninstall.iss
;  or use scripts\build-installer.ps1 (which compiles it automatically).
; ============================================================

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName "Speedy"
; Fixed installation path (matches DefaultDirName in speedy.iss)
#define MyInstallDir "{localappdata}\Programs\Speedy"

[Setup]
AppName={#MyAppName} Uninstaller
AppVersion={#MyAppVersion}
; Registers nothing in the system
CreateUninstallRegKey=no
Uninstallable=no
; Dummy directory — no files are installed
DefaultDirName={tmp}
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
DisableFinishedPage=yes
DisableWelcomePage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=speedy-uninstall-{#MyAppVersion}
WizardStyle=modern
MinVersion=10.0.17763
ArchitecturesAllowed=x64compatible

#ifdef MySetupIcon
SetupIconFile={#MySetupIcon}
#endif

[Languages]
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Code]

{ ----------------------------------------------------------------
  InitializeSetup is called before any page is shown.
  By returning False we abort the wizard immediately (no page is
  displayed), so this exe does not "install" anything.
  ---------------------------------------------------------------- }
function InitializeSetup(): Boolean;
var
  UninstExe: string;
  Params: string;
  ResultCode: Integer;
begin
  Result := False; { Never proceed with the installation }

  UninstExe := ExpandConstant('{localappdata}\Programs\Speedy\unins000.exe');

  if not FileExists(UninstExe) then
  begin
    if not WizardSilent() then
      MsgBox(
        'Speedy does not appear to be installed on this system.' + #13#10 +
        'File not found: ' + UninstExe,
        mbError, MB_OK);
    Exit;
  end;

  { Propagate silent mode to the real uninstaller }
  Params := '';
  if WizardSilent() then
    Params := '/SILENT /SUPPRESSMSGBOXES';

  if not Exec(UninstExe, Params, '', SW_SHOW, ewWaitUntilTerminated, ResultCode) then
  begin
    if not WizardSilent() then
      MsgBox(
        'Unable to start the uninstall program.' + #13#10 +
        'Path: ' + UninstExe + #13#10 +
        'Error code: ' + IntToStr(ResultCode),
        mbError, MB_OK);
  end;
end;
