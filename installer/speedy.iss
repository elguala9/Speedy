; ============================================================
;  Speedy — Inno Setup 6 installer script
;
;  Build minimo:
;    iscc /DMyAppVersion=0.1.0 installer\speedy.iss
;
;  Build con icona custom:
;    iscc /DMyAppVersion=0.1.0 /DMySetupIcon=installer\assets\speedy.ico installer\speedy.iss
;
;  Usa scripts\build-installer.ps1 per fare tutto in un colpo.
; ============================================================

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName      "Speedy"
#define MyAppPublisher "Omnia Group"
#define MyAppURL       "https://github.com/elguala9/Speedy"
; GUID fisso — non cambiare: identifica l'app per gli aggiornamenti automatici
; (le {{ }} sono escape per le parentesi graffe in Inno Setup)
#define MyAppId        "6F3A1B2C-4D5E-6F7A-8B9C-0D1E2F3A4B5C"

; ============================================================
[Setup]
; ============================================================
AppId={{{#MyAppId}}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases

; Installa in %LOCALAPPDATA%\Programs\Speedy — nessun admin richiesto
DefaultDirName={localappdata}\Programs\Speedy
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes

; Nessun privilegio di amministratore necessario
PrivilegesRequired=lowest

; Output
OutputDir=..\dist
OutputBaseFilename=speedy-setup-{#MyAppVersion}

; Icona custom (opzionale — passa /DMySetupIcon=percorso al .ico per usarla)
#ifdef MySetupIcon
SetupIconFile={#MySetupIcon}
#endif

; Compressione massima (riduce le dimensioni dell'installer del 30-40%)
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern

; Notifica Windows del cambio PATH così i nuovi terminali lo vedono subito
ChangesEnvironment=yes

; Durante un aggiornamento, chiedi di chiudere i processi Speedy in esecuzione
CloseApplications=yes
CloseApplicationsFilter=speedy-daemon.exe,speedy-gui.exe,speedy-ai-context.exe,speedy-language-context.exe

; Uninstaller
UninstallDisplayName={#MyAppName} {#MyAppVersion}
UninstallDisplayIcon={app}\speedy-gui.exe

; Requisito minimo: Windows 10 1809 (che ha Unix domain socket nativo)
MinVersion=10.0.17763
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; ============================================================
[Languages]
; ============================================================
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

; ============================================================
[Tasks]
; ============================================================
; Task selezionati di default (Flags: checkedonce = selezionato la prima volta, poi ricorda la scelta)
Name: "autostart";   Description: "Avvia il daemon automaticamente ad ogni login (consigliato)";  GroupDescription: "Avvio automatico:";    Flags: checkedonce
Name: "addtopath";   Description: "Aggiungi Speedy al PATH (richiesto da speedy-cli e speedy-mcp)"; GroupDescription: "Integrazione sistema:"; Flags: checkedonce

; Task non selezionato di default
Name: "desktopicon"; Description: "Crea collegamento sul Desktop per Speedy GUI"; GroupDescription: "Icone aggiuntive:"; Flags: unchecked

; ============================================================
[Files]
; ============================================================
; Binari principali (ignoreversion = sovrascrive sempre, utile per aggiornamenti)
Source: "..\dist\speedy-ai-context.exe";        DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\speedy-daemon.exe";            DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\speedy-cli.exe";               DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\speedy-ai-context-mcp.exe";               DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\speedy-gui.exe";               DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\speedy-language-context.exe";      DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\speedy-language-context-mcp.exe"; DestDir: "{app}"; Flags: ignoreversion


; ============================================================
[Icons]
; ============================================================

; --- Avvio automatico al login (task: autostart) ---
; speedy-daemon.exe è compilato con windows_subsystem="windows" in release build,
; quindi parte senza finestra console — nessun wrapper VBS necessario.
Name: "{userstartup}\Speedy Daemon"; \
  Filename: "{app}\speedy-daemon.exe"; \
  WorkingDir: "{app}"; \
  Comment: "Speedy semantic search daemon — avvio automatico"; \
  Tasks: autostart

; --- Start Menu ---
Name: "{group}\Speedy";             Filename: "{app}\speedy-gui.exe";    WorkingDir: "{app}"
Name: "{group}\Speedy Daemon";      Filename: "{app}\speedy-daemon.exe"; WorkingDir: "{app}"
Name: "{group}\Disinstalla Speedy"; Filename: "{uninstallexe}"

; --- Desktop (opzionale) ---
Name: "{userdesktop}\Speedy"; Filename: "{app}\speedy-gui.exe"; WorkingDir: "{app}"; Tasks: desktopicon

; ============================================================
[Registry]
; ============================================================

; Aggiunge {app} al PATH utente (REG_EXPAND_SZ in HKCU\Environment\Path).
; {olddata} = valore attuale del registry.
; La funzione Check: NeedsAddPath evita duplicati.
; uninsneveruninstall: Inno Setup NON ripristina il vecchio valore PATH durante
; la disinstallazione — lo fa RemoveFromPath nel [Code], che rimuove solo il
; segmento Speedy invece di ripristinare un valore potenzialmente obsoleto.
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; \
  ValueData: "{olddata};{app}"; \
  Tasks: addtopath; \
  Check: NeedsAddPath(ExpandConstant('{app}')); \
  Flags: noerror

; ============================================================
[Run]
; ============================================================

; Avvia il daemon subito al termine dell'installazione (senza aspettare il prossimo login).
; runhidden: non apre una finestra (supportato da windows_subsystem in release build).
; Nota: non ha il flag postinstall, quindi gira automaticamente senza mostrarlo come checkbox.
Filename: "{app}\speedy-daemon.exe"; \
  WorkingDir: "{app}"; \
  Flags: nowait runhidden; \
  StatusMsg: "Avvio daemon in background..."

; Offre all'utente di aprire la GUI al termine (checkbox nella pagina Finish).
Filename: "{app}\speedy-gui.exe"; \
  WorkingDir: "{app}"; \
  Description: "Apri Speedy GUI"; \
  Flags: nowait postinstall shellexec

; ============================================================
[UninstallDelete]
; ============================================================

; La cartella logs/ viene creata a runtime (non è in [Files]) — la eliminiamo sempre.
Type: filesandordirs; Name: "{app}\logs"

; ============================================================
[UninstallRun]
; ============================================================

; 1. Stop graceful del daemon via IPC.
; skipifdoesntexist: tollera installazioni parziali.
; waituntilterminated: aspetta che speedy-cli.exe finisca prima di procedere.
Filename: "{app}\speedy-cli.exe"; \
  Parameters: "daemon stop"; \
  Flags: runhidden skipifdoesntexist waituntilterminated; \
  RunOnceId: "StopDaemon"; \
  StatusMsg: "Arresto Speedy Daemon in corso..."

; 2. Force-kill dei processi rimasti (in caso il daemon non risponda all'IPC).
; taskkill esce con errore se il processo non e' in esecuzione — ignorato da Inno Setup.
Filename: "{sys}\taskkill.exe"; \
  Parameters: "/F /IM speedy-daemon.exe /T"; \
  Flags: runhidden; \
  RunOnceId: "KillDaemon"; \
  StatusMsg: "Chiusura forzata processi Speedy..."

Filename: "{sys}\taskkill.exe"; \
  Parameters: "/F /IM speedy-gui.exe /T"; \
  Flags: runhidden; \
  RunOnceId: "KillGui"

Filename: "{sys}\taskkill.exe"; \
  Parameters: "/F /IM speedy-ai-context.exe /T"; \
  Flags: runhidden; \
  RunOnceId: "KillAiCtx"

Filename: "{sys}\taskkill.exe"; \
  Parameters: "/F /IM speedy-language-context.exe /T"; \
  Flags: runhidden; \
  RunOnceId: "KillLangCtx"

; ============================================================
[Code]

var
  ShouldDeleteUserData: Boolean;

{ ----------------------------------------------------------------
  PATH — NeedsAddPath
  Restituisce True se AppPath non è già presente nel PATH utente.
  Confronto case-insensitive: aggiunge ;AppPath in mezzo al PATH.
  ---------------------------------------------------------------- }
function NeedsAddPath(AppPath: string): Boolean;
var
  CurrentPath: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', CurrentPath) then
  begin
    Result := True;
    Exit;
  end;
  { Circoscrive con ';' per evitare match parziali (es. \Speedy vs \SpeedyExtra) }
  Result := Pos(';' + Uppercase(AppPath) + ';',
                ';' + Uppercase(CurrentPath) + ';') = 0;
end;

{ ----------------------------------------------------------------
  PATH — RemoveFromPath
  Rimuove AppPath dal PATH utente (case-insensitive).
  Gestisce sia ';AppPath' sia 'AppPath;' all'inizio/fine.
  ---------------------------------------------------------------- }
procedure RemoveFromPath(AppPath: string);
var
  CurrentPath, AppUC, CurUC: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', CurrentPath) then
    Exit;

  AppUC := Uppercase(AppPath);
  CurUC := Uppercase(CurrentPath);

  { Cerca ';AppPath' (caso più comune: AppPath in mezzo o alla fine) }
  P := Pos(';' + AppUC, CurUC);
  if P > 0 then
  begin
    Delete(CurrentPath, P, 1 + Length(AppPath));
    RegWriteExpandStringValue(HKCU, 'Environment', 'Path', CurrentPath);
    Exit;
  end;

  { Cerca 'AppPath;' (caso: AppPath all'inizio del PATH) }
  P := Pos(AppUC + ';', CurUC);
  if P > 0 then
  begin
    Delete(CurrentPath, P, Length(AppPath) + 1);
    RegWriteExpandStringValue(HKCU, 'Environment', 'Path', CurrentPath);
  end;
end;

{ ----------------------------------------------------------------
  UNINSTALL — InitializeUninstall
  Mostra un dialog che chiede se eliminare anche i dati utente.
  Default: No (MB_DEFBUTTON2) per sicurezza.
  ---------------------------------------------------------------- }
function InitializeUninstall(): Boolean;
var
  Answer: Integer;
  AppData, UserProfile: string;
begin
  Result    := True;
  AppData   := GetEnv('APPDATA');
  UserProfile := GetEnv('USERPROFILE');

  Answer := MsgBox(
    'Vuoi eliminare anche tutti i dati utente di Speedy?' + #13#10 +
    '' + #13#10 +
    'Verranno eliminati:' + #13#10 +
    '  ' + AppData + '\speedy\' + #13#10 +
    '    (workspaces registrati, log del daemon, configurazione)' + #13#10 +
    '  ' + AppData + '\Speedy GUI\' + #13#10 +
    '    (preferenze e layout della GUI)' + #13#10 +
    '  ' + UserProfile + '\.speedy\' + #13#10 +
    '    (configurazione globale utente)' + #13#10 +
    '' + #13#10 +
    'NON vengono toccate le cartelle .speedy\ nei tuoi progetti' + #13#10 +
    '(indici e database restano intatti).' + #13#10 +
    '' + #13#10 +
    'Si = rimozione completa.  No = mantieni i dati (consigliato).',
    mbConfirmation,
    MB_YESNO or MB_DEFBUTTON2
  );
  ShouldDeleteUserData := (Answer = IDYES);
end;

{ ----------------------------------------------------------------
  UNINSTALL - CurUninstallStepChanged
  usUninstall:     rimuove la dir di installazione dal PATH utente
                   e cancella le chiavi di registro specifiche di Speedy
  usPostUninstall: se l'utente ha scelto Si, cancella le dir dati
  ---------------------------------------------------------------- }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  AppData, LocalAppData, UserProfile: string;
  ResultCode: Integer;
begin
  case CurUninstallStep of

    usUninstall:
    begin
      // Kill difensivo: se i processi fossero ancora vivi bloccano i file.
      // ResultCode ignorato (taskkill esce con errore se il processo non gira).
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-daemon.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-gui.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-ai-context.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-language-context.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Sleep(1200); // attende che il SO rilasci i file lock prima della cancellazione

      // Rimuove solo il segmento {app} dal PATH, non tocca il resto
      RemoveFromPath(ExpandConstant('{app}'));

      { Chiavi di registro specifiche di Speedy (no-op se non esistono) }
      RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Speedy');
      RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Omnia Group\Speedy');
      { Rimuove il parent solo se rimasto vuoto (altri prodotti Omnia potrebbero esserci) }
      RegDeleteKeyIfEmpty(HKCU, 'Software\Omnia Group');
    end;

    usPostUninstall:
    begin
      if not ShouldDeleteUserData then
        Exit;

      AppData     := GetEnv('APPDATA');
      LocalAppData := GetEnv('LOCALAPPDATA');
      UserProfile := GetEnv('USERPROFILE');

      if AppData <> '' then
      begin
        { Dati del daemon: workspaces.json, logs/, daemon.pid, daemon.lock }
        DelTree(AppData + '\speedy', True, True, True);

        { Dati GUI (eframe "Speedy GUI"): preferenze, layout, socket name }
        DelTree(AppData + '\Speedy GUI', True, True, True);
      end;

      if LocalAppData <> '' then
      begin
        { Cache eframe (alcune versioni scrivono qui invece di AppData) }
        DelTree(LocalAppData + '\Speedy GUI', True, True, True);
      end;

      if UserProfile <> '' then
      begin
        { Configurazione globale utente: ~/.speedy/config.speedy.json }
        DelTree(UserProfile + '\.speedy', True, True, True);
      end;
    end;

  end;
end;
