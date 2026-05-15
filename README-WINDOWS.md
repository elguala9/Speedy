# Installazione su Windows

Guida passo-passo per installare i 5 binari di Speedy su Windows e configurare l'avvio automatico del daemon.

---

## Prerequisiti

1. **Ollama** — [ollama.com](https://ollama.com) — deve girare in background.
2. Scarica il modello di embedding (una volta sola):
   ```powershell
   ollama pull all-minilm:l6-v2
   ```

---

## I 5 binari e i loro ruoli

| Binario | Ruolo | Lifecycle |
|---|---|---|
| `speedy-ai-context.exe` | Worker — indexing, query, embedding, SQLite | Lanciato dal daemon, o standalone |
| `speedy-daemon.exe` | Daemon globale — file watcher, IPC server | **Sempre in esecuzione** (uno per utente) |
| `speedy-cli.exe` | Client thin — scriptabile, per AI agent | Lanciato on-demand |
| `speedy-mcp.exe` | Server MCP per AI agent (Claude, Cursor, …) | Lanciato dall'agent MCP |
| `speedy-gui.exe` | GUI desktop — gestione workspaces e log | Lanciato on-demand |

I primi quattro (`speedy-ai-context`, `speedy-cli`, `speedy-mcp`, `speedy-gui`) si lanciano e terminano in secondi. Il daemon è l'unico che deve restare sempre vivo.

---

## Layout consigliato

```
C:\Program Files\Speedy\        ← o %LOCALAPPDATA%\Programs\Speedy\ senza admin
├── speedy-ai-context.exe
├── speedy-daemon.exe
├── speedy-cli.exe
├── speedy-mcp.exe
└── speedy-gui.exe
```

Tutti e 5 nella stessa cartella: così `speedy-gui.exe` trova il daemon automaticamente via auto-detect (cerca `speedy-daemon.exe` accanto a sé).

---

## Step 1 — Copia i binari

Scarica i `.exe` dalla [pagina Releases](https://github.com/elguala9/Speedy/releases) (o buildali con `cargo build --release --workspace`) e copiamoli nella cartella:

```powershell
$dir = 'C:\Program Files\Speedy'
New-Item -ItemType Directory -Force $dir

Copy-Item dist\speedy-ai-context.exe $dir
Copy-Item dist\speedy-daemon.exe $dir
Copy-Item dist\speedy-cli.exe    $dir
Copy-Item dist\speedy-mcp.exe    $dir
Copy-Item dist\speedy-gui.exe    $dir
```

> Se non hai admin rights usa `$dir = "$env:LOCALAPPDATA\Programs\Speedy"`.

---

## Step 2 — Aggiungi la cartella al PATH

Facoltativo ma consigliato: permette di scrivere `speedy-cli daemon status` da qualsiasi shell e di referenziare `speedy-mcp` negli MCP client senza percorso assoluto.

```powershell
$dir = 'C:\Program Files\Speedy'
[Environment]::SetEnvironmentVariable(
    'Path',
    [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + $dir,
    'User')
```

Apri un nuovo terminale dopo.

---

## Step 3 — Autostart del daemon al login

`speedy-daemon.exe` deve partire automaticamente ad ogni login. Il modo più semplice è creare uno **shortcut nella cartella Startup** dell'utente.

Apri la cartella Startup: `Win + R` → digita `shell:startup` → Invio.

Poi crea lo shortcut via PowerShell:

```powershell
$startup = [Environment]::GetFolderPath('Startup')
$target  = 'C:\Program Files\Speedy\speedy-daemon.exe'
$ws      = New-Object -ComObject WScript.Shell
$lnk     = $ws.CreateShortcut("$startup\speedy-daemon.lnk")
$lnk.TargetPath = $target
$lnk.Save()
```

Al prossimo login il daemon parte in background senza finestra (flag `CREATE_NO_WINDOW` già attivo). Per avviarlo subito senza rilogarsi:

```powershell
Start-Process 'C:\Program Files\Speedy\speedy-daemon.exe' -WindowStyle Hidden
```

---

## Step 4 — Verifica

```powershell
speedy-cli daemon ping      # deve rispondere: pong
speedy-cli daemon status    # JSON con pid, uptime, watcher_count
```

---

## Step 5 — Primo workspace

```powershell
# Registra il progetto (il daemon avvia un file watcher)
speedy-cli workspace add C:\path\to\project

# Indicizza
speedy-cli index

# Cerca
speedy-cli query "come funziona l'autenticazione?" -k 10
```

In alternativa apri `speedy-gui.exe` → tab **Workspaces** → **Aggiungi**.

---

## Configurare un AI agent (MCP)

Aggiungi al config del tuo client MCP (es. `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "speedy": {
      "command": "C:\\Program Files\\Speedy\\speedy-mcp.exe",
      "args": [],
      "env": {
        "SPEEDY_BIN": "speedy-cli",
        "SPEEDY_DEFAULT_SOCKET": "speedy-daemon",
        "SPEEDY_MCP_TOP_K": "10"
      }
    }
  }
}
```

> Se hai aggiunto la cartella al PATH puoi usare solo `"command": "speedy-mcp"`.

---

## Rimozione

```powershell
# Rimuovi lo shortcut di autostart
Remove-Item "$([Environment]::GetFolderPath('Startup'))\speedy-daemon.lnk" -ErrorAction SilentlyContinue

# Ferma il daemon (se è in esecuzione)
speedy-cli daemon stop

# Elimina i binari
Remove-Item -Recurse -Force 'C:\Program Files\Speedy'
```

Il database dei workspace (`.speedy/index.sqlite`) vive dentro ogni progetto e non viene toccato da questa procedura.
