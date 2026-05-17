; ============================================================
;  Speedy — Uninstaller runner
;
;  Non installa nulla. Quando eseguito, cerca il vero uninstaller
;  di Speedy in %LOCALAPPDATA%\Programs\Speedy\ e lo lancia.
;
;  Build:
;    iscc /DMyAppVersion=0.1.0 installer\speedy-uninstall.iss
;  oppure usa scripts\build-installer.ps1 (lo compila automaticamente).
; ============================================================

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName "Speedy"
; Percorso fisso di installazione (coincide con DefaultDirName in speedy.iss)
#define MyInstallDir "{localappdata}\Programs\Speedy"

[Setup]
AppName={#MyAppName} Uninstaller
AppVersion={#MyAppVersion}
; Non registra nulla nel sistema
CreateUninstallRegKey=no
Uninstallable=no
; Directory fittizia — nessun file viene installato
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
  InitializeSetup è chiamata prima di mostrare qualsiasi pagina.
  Restituendo False abbandoniamo subito il wizard (nessuna pagina
  viene visualizzata), quindi questo exe non "installa" nulla.
  ---------------------------------------------------------------- }
function InitializeSetup(): Boolean;
var
  UninstExe: string;
  Params: string;
  ResultCode: Integer;
begin
  Result := False; { Non procedere mai con l'installazione }

  UninstExe := ExpandConstant('{localappdata}\Programs\Speedy\unins000.exe');

  if not FileExists(UninstExe) then
  begin
    if not WizardSilent() then
      MsgBox(
        'Speedy non risulta installato su questo sistema.' + #13#10 +
        'File non trovato: ' + UninstExe,
        mbError, MB_OK);
    Exit;
  end;

  { Propaga la modalità silenziosa al vero uninstaller }
  Params := '';
  if WizardSilent() then
    Params := '/SILENT /SUPPRESSMSGBOXES';

  if not Exec(UninstExe, Params, '', SW_SHOW, ewWaitUntilTerminated, ResultCode) then
  begin
    if not WizardSilent() then
      MsgBox(
        'Impossibile avviare il programma di disinstallazione.' + #13#10 +
        'Percorso: ' + UninstExe + #13#10 +
        'Codice errore: ' + IntToStr(ResultCode),
        mbError, MB_OK);
  end;
end;
