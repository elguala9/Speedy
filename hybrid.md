# Speedy — Piano Ibrido: Daemon + Git Hooks

**Obiettivo**: i git hooks coprono il caso "daemon spento" e accelerano la sincronizzazione post-commit anche quando il daemon è attivo, senza duplicare lavoro.

---

## Architettura

```
 git commit / checkout / merge
          │
          ▼
    hook script (sh/ps1)
          │
          ├─── daemon UP? ──YES──► speedy-ai-context daemon exec index <file> (IPC)
          │                         (daemon già osserva il FS, ma il hook
          │                          forza l'index immediato senza attendere
          │                          il debounce da 500 ms)
          │
          └─── daemon DOWN? ────► speedy-ai-context index <file>  (processo diretto)
                                   (fallback senza IPC, nessun daemon richiesto)
```

Il daemon rimane il percorso principale per modifiche non committate (salvataggi continui, refactor live). I hook sono l'acceleratore e il safety-net.

---

## File da creare

### 1. Hook scripts — `scripts/git-hooks/`

**`post-commit`** (template sh — `{{SPEEDY_EXE}}` sostituito da `install-hooks`)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
SPEEDY="{{SPEEDY_EXE}}"
CHANGED=$(git diff-tree --no-commit-id -r --name-only HEAD 2>/dev/null)
[ -z "$CHANGED" ] && exit 0

ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY" ping 2>/dev/null | grep -q "pong"; then
    # Daemon up: notify via IPC (fast path)
    for f in $CHANGED; do
        [ -f "$ROOT/$f" ] && "$SPEEDY" daemon exec -- index "$f"
    done
else
    # Daemon down: index direttamente
    for f in $CHANGED; do
        [ -f "$ROOT/$f" ] && SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" index "$f"
    done
fi
exit 0
```

**`post-checkout`** (template sh)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
SPEEDY="{{SPEEDY_EXE}}"
# $3 = 1 se branch switch, 0 se file checkout
[ "$3" = "0" ] && exit 0

ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY" ping 2>/dev/null | grep -q "pong"; then
    "$SPEEDY" daemon sync "$ROOT"
else
    SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" sync
fi
exit 0
```

**`post-merge`** (template sh)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
SPEEDY="{{SPEEDY_EXE}}"
ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY" ping 2>/dev/null | grep -q "pong"; then
    "$SPEEDY" daemon sync "$ROOT"
else
    SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" sync
fi
exit 0
```

**`post-rewrite`** (template sh — copre rebase e amend)
```sh
#!/bin/sh
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
SPEEDY="{{SPEEDY_EXE}}"
# $1 = "rebase" o "amend"
ROOT=$(git rev-parse --show-toplevel)

if "$SPEEDY" ping 2>/dev/null | grep -q "pong"; then
    "$SPEEDY" daemon reindex "$ROOT"
else
    SPEEDY_NO_DAEMON=1 "$SPEEDY" -p "$ROOT" index .
fi
exit 0
```

**`post-commit.ps1`** (template PowerShell — alternativa Windows nativa)
```powershell
# Speedy — managed hook (do not edit — reinstall with: speedy-ai-context install-hooks)
$SPEEDY = "{{SPEEDY_EXE}}"
$changed = git diff-tree --no-commit-id -r --name-only HEAD 2>$null
if (-not $changed) { exit 0 }

$root = git rev-parse --show-toplevel

$daemonUp = (& $SPEEDY ping 2>$null) -eq "pong"

foreach ($f in $changed) {
    $full = Join-Path $root $f
    if (Test-Path $full) {
        if ($daemonUp) {
            & $SPEEDY daemon exec -- index $f
        } else {
            $env:SPEEDY_NO_DAEMON = "1"
            & $SPEEDY -p $root index $f
        }
    }
}
exit 0
```

---

### 2. Nuovo comando CLI — `speedy-ai-context install-hooks` / `speedy-ai-context uninstall-hooks`

**`packages/speedy-ai-context/src/hooks.rs`** (nuovo file)

Responsabilità:
- Risolve la cartella `.git/hooks/` del repo corrente (o via `git rev-parse --git-path hooks`)
- Ottiene il path assoluto del proprio eseguibile via `std::env::current_exe()` e lo **interpola nei template degli hook** — gli script non chiamano `speedy-ai-context` nudo ma il path esatto del binario che ha eseguito `install-hooks`
- Rende gli script eseguibili (`chmod +x` su Unix, noop su Windows)
- `uninstall-hooks`: rimuove solo i file che hanno il marker `# Speedy — managed hook` in cima
- Stampa un report: quali hook installati, dove, se ne ha trovati di preesistenti

**Perché non `include_str!` verbatim**: i template hanno un placeholder `{{SPEEDY_EXE}}` che viene sostituito a runtime con il path assoluto risolto da `current_exe()`. Questo garantisce che il hook funzioni anche se `speedy-ai-context` non è in `PATH` (installazione locale, `cargo install --path`, path custom).

```rust
// hooks.rs — logica centrale
let exe = std::env::current_exe()?.canonicalize()?;
let script = HOOK_POST_COMMIT_TEMPLATE
    .replace("{{SPEEDY_EXE}}", &exe.to_string_lossy());
std::fs::write(&hook_path, script)?;
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
}
```

**Modifica `packages/speedy-ai-context/src/cli.rs`**:
```rust
InstallHooks {
    /// Path del repo (default: CWD)
    #[arg(long)]
    path: Option<PathBuf>,
    /// Usa symlink invece di copia (più comodo per sviluppo)
    #[arg(long)]
    symlink: bool,
},
UninstallHooks {
    #[arg(long)]
    path: Option<PathBuf>,
},
```

---

## File da modificare

### `packages/speedy-core/src/config.rs`

Aggiungere campo opzionale (default `true`):
```rust
pub hooks_enabled: bool,   // default: true
```

Usato da `install-hooks` per decidere se fare skip e da eventuali warning.

---

### `packages/speedy-daemon/src/main.rs`

**Nuovo IPC command: `ping`** — già esiste (`ping` → `pong`), usato dagli hook per check daemon-up. **Nessuna modifica necessaria** per questo.

**Nuovo IPC command: `notify-commit\t<path>\t<file1>\t<file2>...`** (opzionale, fase 2):
- Più efficiente di mandare N richieste `exec index <file>` separate
- Riceve una lista di file, li accoda all'indexer del workspace senza passare per subprocess
- Handler in `dispatch_command()`, circa riga 887

Per la fase 1 basta riusare `exec -- index <file>` già esistente (riga 1022-1026).

---

## Flusso di installazione utente

```
# 1. Registra il workspace (già esistente)
speedy-ai-context add .

# 2. Installa gli hook nel repo corrente
speedy-ai-context install-hooks

# Output atteso:
# ✓ Installed post-commit    → .git/hooks/post-commit
# ✓ Installed post-checkout  → .git/hooks/post-checkout
# ✓ Installed post-merge     → .git/hooks/post-merge
# ✓ Installed post-rewrite   → .git/hooks/post-rewrite
# Tip: run `speedy-ai-context uninstall-hooks` to remove them.
```

---

## Embedding dei template nel binario

I template vengono embeddati in `speedy-ai-context` con `include_str!` a compile time. Hanno il placeholder `{{SPEEDY_EXE}}` che viene sostituito con il path assoluto a runtime:

```rust
// packages/speedy-ai-context/src/hooks.rs
const HOOK_POST_COMMIT_TPL:   &str = include_str!("../../scripts/git-hooks/post-commit.tpl");
const HOOK_POST_CHECKOUT_TPL: &str = include_str!("../../scripts/git-hooks/post-checkout.tpl");
const HOOK_POST_MERGE_TPL:    &str = include_str!("../../scripts/git-hooks/post-merge.tpl");
const HOOK_POST_REWRITE_TPL:  &str = include_str!("../../scripts/git-hooks/post-rewrite.tpl");
// Windows
const HOOK_POST_COMMIT_PS1_TPL: &str = include_str!("../../scripts/git-hooks/post-commit.ps1.tpl");
```

Scrittura a disco:
```rust
let exe = std::env::current_exe()?.canonicalize()?;
// su Windows Git-Bash il path deve essere in formato POSIX: /c/Users/...
let exe_str = normalize_for_sh(&exe);
let script = TPL.replace("{{SPEEDY_EXE}}", &exe_str);
```

Su Windows si scrive sia lo script `.sh` (usato da Git-Bash) sia un `.bat` wrapper che invoca PowerShell per chi usa CMD.

---

## Edge cases da gestire

| Caso | Comportamento |
|---|---|
| Hook preesistente (non-Speedy) | `install-hooks` stampa warning e chiede conferma prima di sovrascrivere |
| `core.hooksPath` globale | Rispettato: `git rev-parse --git-path hooks` restituisce il path corretto |
| Repo senza `.speedy/` | Gli hook si installano lo stesso; al run faranno `speedy-ai-context index` che crea `.speedy/` |
| `--no-verify` | Bypassa i hook: documentare come limitazione nota |
| Submoduli | Gli hook vanno installati per-submodulo; `install-hooks --recursive` come flag fase 2 |
| CI/CD (GitHub Actions, ecc.) | `SPEEDY_SKIP_HOOKS=1` env var fa exit 0 immediato in tutti gli hook |

---

## Fasi di sviluppo

### Fase 1 — MVP (priorità alta)
1. Creare `scripts/git-hooks/post-commit`, `post-checkout`, `post-merge`, `post-rewrite`
2. Creare `packages/speedy-ai-context/src/hooks.rs` con install/uninstall logic
3. Aggiungere `InstallHooks` / `UninstallHooks` a `packages/speedy-ai-context/src/cli.rs` e `main.rs`
4. Test manuale su Windows (Git-Bash) e Linux

### Fase 2 — Ottimizzazioni
5. IPC command `notify-commit` per batch di file (evita N subprocess)
6. Flag `--recursive` per submoduli
7. `SPEEDY_SKIP_HOOKS` env var
8. Hook per PowerShell nativo (`.ps1`) con `.bat` wrapper su Windows

### Fase 3 — UX
9. `speedy-ai-context add .` installa gli hook automaticamente se `hooks_enabled = true`
10. `speedy-ai-context status` mostra se gli hook sono installati per il repo corrente

---

## Dipendenze nuove

Nessuna. Tutto il codice necessario è già disponibile:
- `speedy-ai-context ping` (IPC) — già esiste
- `speedy-ai-context daemon exec` (IPC) — già esiste (riga 1022-1026 daemon/main.rs)
- `speedy-ai-context daemon sync` (IPC) — già esiste (riga 979-986)
- `speedy-ai-context daemon reindex` (IPC) — già esiste (riga 988-995)
- `speedy-ai-context index <file>` (diretto) — già esiste
