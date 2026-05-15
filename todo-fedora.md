# Speedy su Fedora — TODO

Portare lo stesso flusso "build + install + autostart" che oggi è documentato
solo per Windows anche su Fedora (e per estensione Linux generico). Il
workspace già compila (target unix, niente API Windows-only nei path attivi
— `winreg`/`uds_windows` sono dietro `cfg(windows)`), quindi il lavoro è
**packaging, documentazione e integrazione desktop**, non codice nuovo di
funzionalità.

---

## 1. Verifica build effettiva su Fedora

Prima di promettere "compila ovunque" serve provarlo davvero.

- [ ] Build pulita su Fedora 41+ con `rustup` stabile:
      ```
      cargo build --release --workspace
      ```
      Annotare pacchetti `dnf` davvero necessari (sotto è una lista plausibile,
      va validata).
- [ ] `cargo test --workspace` verde. Attenzione ai test daemon che
      serializzano su mutex globale (potrebbero richiedere
      `-- --test-threads=1`).
- [ ] Lancio interattivo `cargo run --release -p speedy-gui` su:
  - [ ] GNOME / Wayland (tray icon richiede estensione AppIndicator).
  - [ ] KDE Plasma / Wayland (tray nativo).
  - [ ] X11 fallback.
- [ ] Verifica che `interprocess` su Linux usi UDS in
      `$XDG_RUNTIME_DIR/speedy-daemon` (o equivalente) e che il path sia
      stabile tra logout/login.

### Pacchetti `dnf` candidati

```
gcc pkgconf-pkg-config
glib2-devel gtk3-devel
libxkbcommon-devel libxcb-devel wayland-devel
libappindicator-gtk3-devel        # serve a tray-icon
openssl-devel
```

`rusqlite` ha `features = ["bundled"]` → niente `sqlite-devel`.
`reqwest` usa rustls per default? Verificare il `Cargo.lock` per capire se
serve `openssl-devel` davvero (se è solo rustls, si può togliere).

---

## 2. Script di build Linux già esistente

`scripts/build-release.sh` esiste e copia in `dist/`. Da rifinire:

- [ ] Aggiungere check upfront delle dipendenze di sistema (un `pkg-config
      --exists gtk+-3.0 ayatana-appindicator3-0.1` con messaggio chiaro se
      mancano), così l'errore non è un wall of text di `cargo`.
- [ ] Stampare la lista dei 5 binari prodotti con dimensioni (cosmetico).

---

## 3. Documentazione README — sezione Linux

Oggi il README ha solo la sezione Windows ("Recommended layout (Windows)",
"Startup folder"). Aggiungere paragrafo gemello.

- [ ] **Install path consigliato**: `~/.local/bin/` (già su `PATH` di default
      su Fedora) per i 4 binari front-end (`speedy`, `speedy-cli`,
      `speedy-mcp`, `speedy-gui`). Mantenere `speedy-daemon` separato (vedi
      autostart sotto).
- [ ] **Comando di copia** equivalente al blocco PowerShell di Windows:
      ```
      install -Dm755 dist/speedy        ~/.local/bin/speedy
      install -Dm755 dist/speedy-cli    ~/.local/bin/speedy-cli
      install -Dm755 dist/speedy-mcp    ~/.local/bin/speedy-mcp
      install -Dm755 dist/speedy-gui    ~/.local/bin/speedy-gui
      install -Dm755 dist/speedy-daemon ~/.local/libexec/speedy-daemon
      ```
- [ ] Sezione "**Autostart del daemon**" con i due percorsi (vedi §4).
- [ ] Sezione "**Tray icon su GNOME**" che spiega di installare l'estensione
      *AppIndicator and KStatusNotifierItem Support* (`gnome-extensions`),
      altrimenti la tray non appare. Su KDE/Cinnamon/XFCE funziona out-of-the-box.
- [ ] Aggiornare la tabella dei binari per togliere il suffisso `.exe`
      quando si parla di Linux, o fare due tabelle separate.
- [ ] Path di config: già menziona `~/.config/speedy/` ma è citato di sfuggita,
      promuoverlo a paragrafo nella sezione Linux.

---

## 4. Autostart del daemon su Linux

L'equivalente della Startup folder di Windows. Due opzioni, entrambe
documentate, l'utente sceglie.

### 4.1 Opzione A — systemd user service (consigliata)

- [ ] Scrivere `packaging/linux/speedy-daemon.service`:
      ```
      [Unit]
      Description=Speedy semantic-search daemon (user)
      After=default.target

      [Service]
      Type=simple
      ExecStart=%h/.local/libexec/speedy-daemon
      Restart=on-failure
      RestartSec=5

      [Install]
      WantedBy=default.target
      ```
- [ ] Documentare nel README:
      ```
      mkdir -p ~/.config/systemd/user
      cp packaging/linux/speedy-daemon.service ~/.config/systemd/user/
      systemctl --user daemon-reload
      systemctl --user enable --now speedy-daemon
      ```
- [ ] `loginctl enable-linger $USER` (opzionale, se l'utente vuole il
      daemon attivo anche senza sessione grafica aperta — utile su server,
      probabilmente da NON consigliare per default su desktop).

### 4.2 Opzione B — XDG autostart (.desktop)

Più semplice, parte solo quando l'utente fa login grafico.

- [ ] Scrivere `packaging/linux/speedy-daemon.desktop`:
      ```
      [Desktop Entry]
      Type=Application
      Name=Speedy Daemon
      Exec=%h/.local/libexec/speedy-daemon
      X-GNOME-Autostart-enabled=true
      NoDisplay=true
      ```
- [ ] Documentare il path: `~/.config/autostart/speedy-daemon.desktop`.

---

## 5. Integrazione desktop

- [ ] **`.desktop` per la GUI**: `packaging/linux/speedy-gui.desktop` con
      `Icon=speedy`, `Categories=Development;Utility;` così appare nel menu
      applicazioni di GNOME/KDE.
- [ ] **Icona**: serve un PNG (almeno 256x256) o SVG. Oggi la tray usa
      un'icona generata in codice — va bene per la tray, ma per l'entry
      `.desktop` serve un file installato in
      `~/.local/share/icons/hicolor/256x256/apps/speedy.png` (o equivalente
      a livello di sistema).
- [ ] MIME type per i workspace? Probabilmente no — non apriamo file
      direttamente. Skip salvo richiesta.

---

## 6. Packaging vero e proprio (rinviato, opzionale)

Da decidere se vale lo sforzo o se per ora basta "scarica i binari e
copiali in `~/.local/bin/`".

- [ ] **RPM**: spec file in `packaging/rpm/speedy.spec`, build con
      `rpmbuild` o `cargo-generate-rpm`. Vantaggio: installa tutto in
      `/usr/bin`, `/usr/libexec`, `/usr/share/applications` e gestisce le
      dipendenze (`Requires: ollama` — anche se Ollama non è sempre in
      repo Fedora ufficiali).
- [ ] **COPR**: una volta che lo spec funziona, pubblicare su
      `copr.fedorainfracloud.org` per `dnf install speedy` diretto.
- [ ] **AppImage**: alternativa a RPM, single-file, gira anche fuori Fedora.
      Probabilmente overkill — un tarball di binari statici è equivalente.

---

## 7. CI

- [ ] Aggiungere un job a GitHub Actions su `ubuntu-latest` (sufficientemente
      vicino a Fedora per la compilazione, anche se la conferma su Fedora
      vera va fatta a mano) che esegue `cargo build --release --workspace`
      con le dipendenze sopra installate via `apt`.
- [ ] Eventualmente un job specifico `fedora:latest` in container
      (`container: fedora:41`) per avere garanzia esatta. Più lento, da
      valutare.

---

## 8. Riferimenti rapidi

- Cargo workspace root: `Cargo.toml`
- Script build attuale: `scripts/build-release.sh`
- Path di config su Linux: `~/.config/speedy/` (`workspaces.json`, `daemon.pid`)
- Socket UDS: gestito da `interprocess` in `packages/speedy-core/`
- Documentazione IPC: `docs/ipc-protocol.md`
