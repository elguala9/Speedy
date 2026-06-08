; ============================================================
;  Speedy — Inno Setup 6 installer script
;
;  Minimal build:
;    iscc /DMyAppVersion=0.1.0 installer\speedy.iss
;
;  Build with custom icon:
;    iscc /DMyAppVersion=0.1.0 /DMySetupIcon=installer\assets\speedy.ico installer\speedy.iss
;
;  Use scripts\build-installer.ps1 to do everything in one go.
; ============================================================

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName      "Speedy"
#define MyAppPublisher "Parresia"
#define MyAppURL       "https://github.com/elguala9/Speedy"
; Fixed GUID — do not change: identifies the app for automatic updates
; (the {{ }} are escapes for curly braces in Inno Setup)
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

; Installs into %LOCALAPPDATA%\Programs\Speedy — no admin required
DefaultDirName={localappdata}\Programs\Speedy
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes

; No administrator privileges needed
PrivilegesRequired=lowest

; Output
OutputDir=..\dist
OutputBaseFilename=speedy-setup-{#MyAppVersion}

; Custom icon (optional — pass /DMySetupIcon=path to the .ico to use it)
#ifdef MySetupIcon
SetupIconFile={#MySetupIcon}
#endif

; Maximum compression (reduces installer size by 30-40%)
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern

; Notify Windows of the PATH change so new terminals see it immediately
ChangesEnvironment=yes

; We disable Restart Manager: on Windows 11 it can hang during uninstall
; by showing invisible dialogs or waiting for unresponsive processes.
; We kill the processes ourselves via taskkill in the [Run] and [UninstallRun] sections.
CloseApplications=no

; Uninstaller
UninstallDisplayName={#MyAppName} {#MyAppVersion}
UninstallDisplayIcon={app}\speedy-gui.exe

; Minimum requirement: Windows 10 1809 (which has native Unix domain sockets)
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
; Tasks selected by default (Flags: checkedonce = selected the first time, then remembers the choice)
; NB: automatic startup at login is NO LONGER managed by the installer. By default
; Speedy works through git hooks (no daemon). Startup at login is enabled/
; disabled from the GUI (Dashboard → "Start at login").
Name: "addtopath";   Description: "Add Speedy to PATH (required by speedy-cli and speedy-mcp)"; GroupDescription: "System integration:"; Flags: checkedonce

; Task not selected by default
Name: "desktopicon"; Description: "Create a Desktop shortcut for Speedy GUI"; GroupDescription: "Additional icons:"; Flags: unchecked

; Ollama — opt-in: deselected by default, Check hides the task if already installed
Name: "installoollama"; Description: "Download and install Ollama (required for local AI models)"; GroupDescription: "Dependencies:"; Check: OllamaNotInstalled; Flags: unchecked

; Default model — opt-in: deselected by default, Check hides the task if already present
Name: "pullmodel"; Description: "Download the default model nomic-embed-text (~270 MB, requires a connection)"; GroupDescription: "Dependencies:"; Check: ModelNotInstalled; Flags: unchecked

; ============================================================
[Files]
; ============================================================
; Main binaries
;   ignoreversion       — always overwrite (useful for updates)
;   restartreplace      — during install, if the file is in use, schedule for reboot
;   uninsrestartdelete  — during uninstall, if the file is in use, schedule for reboot
;                         (so the uninstaller does not hang on antivirus/RM locks)
Source: "..\dist\speedy-ai-context.exe";        DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-daemon.exe";            DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-cli.exe";               DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-ai-context-mcp.exe";    DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-gui.exe";               DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-language-context.exe";      DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-language-context-mcp.exe";  DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-text-context.exe";          DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "..\dist\speedy-text-context-mcp.exe";      DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete

; Documentation — copied into the installation folder
Source: "..\installer\README.txt";       DestDir: "{app}"; Flags: ignoreversion
Source: "..\installer\INSTALLATION.md";  DestDir: "{app}"; Flags: ignoreversion
Source: "..\installer\FOR-IA.md";        DestDir: "{app}"; Flags: ignoreversion
Source: "..\installer\SETTINGS_MCP.md";  DestDir: "{app}"; Flags: ignoreversion

; Per-MCP usage guides — what to add to AGENT.md / CLAUDE.md for each server
Source: "..\installer\mcp-guides\*"; DestDir: "{app}\mcp-guides"; Flags: ignoreversion recursesubdirs createallsubdirs

; Emergency uninstall script — if the Inno Setup uninstaller
; hangs for any reason, the user can run this script.
Source: "..\installer\uninstall-emergency.ps1"; DestDir: "{app}"; Flags: ignoreversion


; ============================================================
[InstallDelete]
; ============================================================
; Upgrade cleanup: previous versions created a shortcut for automatic
; daemon startup in the Startup folder. Startup at login is now managed
; by the GUI, so we remove any legacy shortcut.
Type: files; Name: "{userstartup}\Speedy Daemon.lnk"

; ============================================================
[Icons]
; ============================================================

; --- Start Menu ---
Name: "{group}\Speedy";             Filename: "{app}\speedy-gui.exe";    WorkingDir: "{app}"
Name: "{group}\Speedy Daemon";      Filename: "{app}\speedy-daemon.exe"; WorkingDir: "{app}"
Name: "{group}\Uninstall Speedy"; Filename: "{uninstallexe}"

; --- Desktop (optional) ---
Name: "{userdesktop}\Speedy"; Filename: "{app}\speedy-gui.exe"; WorkingDir: "{app}"; Tasks: desktopicon

; ============================================================
[Registry]
; ============================================================

; Adds {app} to the user PATH (REG_EXPAND_SZ in HKCU\Environment\Path).
; {olddata} = current registry value.
; The Check: NeedsAddPath function avoids duplicates.
; uninsneveruninstall: Inno Setup does NOT restore the old PATH value during
; uninstall — RemoveFromPath in [Code] does it, removing only the
; Speedy segment instead of restoring a potentially stale value.
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; \
  ValueData: "{olddata};{app}"; \
  Tasks: addtopath; \
  Check: NeedsAddPath(ExpandConstant('{app}')); \
  Flags: noerror

; ============================================================
[Run]
; ============================================================

; The daemon is NO LONGER started automatically at the end of installation.
; By default Speedy works through git hooks (standalone mode, no daemon).
; The user can start the daemon manually from the GUI (Dashboard → "Start daemon")
; and enable startup at login from the same screen.

; Offers the user to open the GUI at the end (checkbox on the Finish page).
Filename: "{app}\speedy-gui.exe"; \
  WorkingDir: "{app}"; \
  Description: "Open Speedy GUI"; \
  Flags: nowait postinstall shellexec

; Offers to open the README with Notepad at the end.
Filename: "{app}\README.txt"; \
  Description: "Open README (usage instructions)"; \
  Flags: nowait postinstall shellexec

; 1. Download OllamaSetup.exe into the installer's temp folder (silent).
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Invoke-WebRequest -Uri 'https://ollama.com/download/OllamaSetup.exe' -OutFile '{tmp}\OllamaSetup.exe' -UseBasicParsing"""; \
  Tasks: installoollama; \
  StatusMsg: "Downloading Ollama..."; \
  Flags: runhidden waituntilterminated

; 2. Opens the Ollama installer in a separate window and waits for completion.
Filename: "{tmp}\OllamaSetup.exe"; \
  Tasks: installoollama; \
  StatusMsg: "Installing Ollama..."; \
  Flags: waituntilterminated

; Downloads the default model nomic-embed-text via Ollama.
; Waits 5s after any Ollama installation, then runs 'ollama pull'.
; If Ollama is not present (user deselected the task above), it exits without errors.
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""$o=[System.Environment]::GetFolderPath('LocalApplicationData')+'\Programs\Ollama\ollama.exe';if(Test-Path $o){{Start-Sleep 5;&$o pull nomic-embed-text}"""; \
  Tasks: pullmodel; \
  StatusMsg: "Downloading model nomic-embed-text (~270 MB)..."; \
  Flags: runhidden waituntilterminated

; ============================================================
[UninstallDelete]
; ============================================================

; The logs/ folder is created at runtime (not in [Files]) — we always delete it.
Type: filesandordirs; Name: "{app}\logs"

; ============================================================
[UninstallRun]
; ============================================================

; Force-kill all Speedy processes.
; NO speedy-cli daemon stop here: it can hang 10s on IPC timeout and adds
; nothing over taskkill /F. taskkill exits with an error if the process is
; not running — ignored by Inno Setup.
Filename: "{sys}\taskkill.exe"; \
  Parameters: "/F /IM speedy-daemon.exe /T"; \
  Flags: runhidden; \
  RunOnceId: "KillDaemon"; \
  StatusMsg: "Closing Speedy processes..."

Filename: "{sys}\taskkill.exe"; \
  Parameters: "/F /IM speedy-cli.exe /T"; \
  Flags: runhidden; \
  RunOnceId: "KillCli"

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

Filename: "{sys}\taskkill.exe"; \
  Parameters: "/F /IM speedy-text-context.exe /T"; \
  Flags: runhidden; \
  RunOnceId: "KillTextCtx"

; ============================================================
[Code]

var
  ShouldDeleteUserData: Boolean;

{ ----------------------------------------------------------------
  OLLAMA — OllamaNotInstalled
  Returns True if ollama.exe is not present in the standard
  installation path (%LOCALAPPDATA%\Programs\Ollama).
  ---------------------------------------------------------------- }
function OllamaNotInstalled(): Boolean;
begin
  Result := not FileExists(GetEnv('LOCALAPPDATA') + '\Programs\Ollama\ollama.exe');
end;

{ ----------------------------------------------------------------
  OLLAMA — ModelNotInstalled
  Returns True if the nomic-embed-text manifest is not present
  in the Ollama models directory (~\.ollama\models\manifests\...).
  ---------------------------------------------------------------- }
function ModelNotInstalled(): Boolean;
begin
  Result := not FileExists(
    GetEnv('USERPROFILE') +
    '\.ollama\models\manifests\registry.ollama.ai\library\nomic-embed-text\latest'
  );
end;

{ ----------------------------------------------------------------
  PATH — NeedsAddPath
  Returns True if AppPath is not already present in the user PATH.
  Case-insensitive comparison: adds ;AppPath within the PATH.
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
  { Bound with ';' to avoid partial matches (e.g. \Speedy vs \SpeedyExtra) }
  Result := Pos(';' + Uppercase(AppPath) + ';',
                ';' + Uppercase(CurrentPath) + ';') = 0;
end;

{ ----------------------------------------------------------------
  PATH — RemoveFromPath
  Removes AppPath from the user PATH (case-insensitive).
  Handles both ';AppPath' and 'AppPath;' at the start/end.
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

  { Look for ';AppPath' (most common case: AppPath in the middle or at the end) }
  P := Pos(';' + AppUC, CurUC);
  if P > 0 then
  begin
    Delete(CurrentPath, P, 1 + Length(AppPath));
    RegWriteExpandStringValue(HKCU, 'Environment', 'Path', CurrentPath);
    Exit;
  end;

  { Look for 'AppPath;' (case: AppPath at the start of the PATH) }
  P := Pos(AppUC + ';', CurUC);
  if P > 0 then
  begin
    Delete(CurrentPath, P, Length(AppPath) + 1);
    RegWriteExpandStringValue(HKCU, 'Environment', 'Path', CurrentPath);
  end;
end;

{ ----------------------------------------------------------------
  UNINSTALL — InitializeUninstall
  Shows a dialog asking whether to also delete user data.
  Default: No (MB_DEFBUTTON2) for safety.
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
    'Do you also want to delete all Speedy user data?' + #13#10 +
    '' + #13#10 +
    'The following will be deleted:' + #13#10 +
    '  ' + AppData + '\speedy\' + #13#10 +
    '    (registered workspaces, daemon logs, configuration)' + #13#10 +
    '  ' + AppData + '\Speedy GUI\' + #13#10 +
    '    (GUI preferences and layout)' + #13#10 +
    '  ' + UserProfile + '\.speedy\' + #13#10 +
    '    (global user configuration)' + #13#10 +
    '' + #13#10 +
    'The .speedy\ folders in your projects are NOT touched' + #13#10 +
    '(indexes and databases remain intact).' + #13#10 +
    '' + #13#10 +
    'Yes = full removal.  No = keep the data (recommended).',
    mbConfirmation,
    MB_YESNO or MB_DEFBUTTON2
  );
  ShouldDeleteUserData := (Answer = IDYES);
end;

{ ----------------------------------------------------------------
  UNINSTALL - CurUninstallStepChanged
  usUninstall:     removes the installation dir from the user PATH
                   and deletes the Speedy-specific registry keys
  usPostUninstall: if the user chose Yes, deletes the data dirs
  ---------------------------------------------------------------- }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  AppData, LocalAppData, UserProfile: string;
  ResultCode: Integer;
begin
  case CurUninstallStep of

    usUninstall:
    begin
      // Defensive kill: if the processes are still alive they lock the files.
      // ResultCode ignored (taskkill exits with an error if the process is not running).
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-daemon.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-gui.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-ai-context.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-language-context.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM speedy-text-context.exe /T',
           '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      Sleep(1200); // waits for the OS to release the file locks before deletion

      // Removes only the {app} segment from the PATH, leaves the rest untouched
      RemoveFromPath(ExpandConstant('{app}'));

      { Speedy-specific registry keys (no-op if they do not exist) }
      RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Speedy');
      RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Parresia\Speedy');
      { Removes the parent only if left empty }
      RegDeleteKeyIfEmpty(HKCU, 'Software\Parresia');

      { Startup-at-login entry created by the GUI (HKCU ...\Run). No-op if absent. }
      RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Speedy Daemon');
      { Legacy automatic startup shortcut (previous versions) }
      DeleteFile(ExpandConstant('{userstartup}\Speedy Daemon.lnk'));
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
        { Daemon data: workspaces.json, logs/, daemon.pid, daemon.lock }
        DelTree(AppData + '\speedy', True, True, True);

        { GUI data (eframe "Speedy GUI"): preferences, layout, socket name }
        DelTree(AppData + '\Speedy GUI', True, True, True);
      end;

      if LocalAppData <> '' then
      begin
        { eframe cache (some versions write here instead of AppData) }
        DelTree(LocalAppData + '\Speedy GUI', True, True, True);
      end;

      if UserProfile <> '' then
      begin
        { Global user configuration: ~/.speedy/config.speedy.json }
        DelTree(UserProfile + '\.speedy', True, True, True);
      end;
    end;

  end;
end;
