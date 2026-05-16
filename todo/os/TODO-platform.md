# TODO — Platform / Hardware

Items che richiedono una macchina fisica o un OS specifico per essere verificati.
Non bloccano il build/CI; vanno fatti manualmente su ogni piattaforma target.

---

## GUI smoke E2E — Windows

`cargo run --release -p speedy-gui` con daemon vivo. Mai eseguito
interattivamente dalla parte 3 del 2026-05-15 in poi.

- [ ] Tray icon appare verde; click destro → Open / Restart / Quit OK.
- [ ] Dashboard: PID/uptime/version visibili; metrics si aggiornano
      dopo un `speedy-cli index` su un workspace di test.
- [ ] Auto-refresh interval DragValue (1–60 s) modifica effettivamente
      la frequenza di poll e persiste dopo restart GUI.
- [ ] Override "Eseguibile daemon": `Sfoglia…` apre file picker;
      `Applica` aggiorna il path; `Ripristina automatico` torna
      all'auto-detect; persistenza dopo restart GUI.
- [ ] Workspaces: add via picker; "Pulisci orfani" rimuove workspace
      il cui path è stato cancellato; badge "⚠ temp" / "⚠ mancante"
      coerenti.
- [ ] Logs: stream `subscribe-log` riceve eventi; switch "File storico"
      carica `daemon.log.YYYY-MM-DD`; export `.json` / `.jsonl` apre
      save-dialog.
- [ ] Notifiche di sistema su `error`: `RUST_LOG=error cargo run …`
      triggera notifica di sistema.

---

## GUI smoke E2E — macOS

Stesso giro della sezione Windows. Mai testato.

- [ ] Build: `cargo build --release -p speedy-gui` (deps: `brew install`
      se serve libxkbcommon o simili — verificare).
- [ ] Tray icon appare (macOS usa `NSStatusItem`; `tray-icon` 0.19
      dovrebbe supportarlo — da confermare).
- [ ] Tutti i punti della checklist Windows ripetuti su macOS.
- [ ] Autostart: shortcut in `~/Library/LaunchAgents/` oppure
      `Login Items` — documentare il metodo consigliato.

---

## GUI smoke E2E — Linux GNOME / KDE

Vedi anche `todo-fedora.md` §1.

- [ ] Build su Fedora 41+: confermare che i pacchetti `dnf` installati
      in CI (`apt-get` è Ubuntu — su Fedora servono nomi diversi, es.
      `gtk3-devel`, `glib2-devel` …).
- [ ] `cargo build --release -p speedy-gui` verde su Fedora 41.
- [ ] Tray icon su GNOME: richiede estensione AppIndicator o
      `libayatana-appindicator` — verificare se `tray-icon` 0.19 la
      usa o se serve workaround.
- [ ] Tray icon su KDE Plasma: dovrebbe funzionare out-of-the-box.
- [ ] File picker (`rfd` 0.14) via GTK — testare.
- [ ] Notifiche (`notify-rust` 4.11) via D-Bus — testare.
- [ ] Tutti i punti della checklist Windows ripetuti.
- [ ] Autostart: una volta creato `speedy-daemon.service` (vedi
      `todo-fedora.md` §4), abilitarlo con
      `systemctl --user enable --now speedy-daemon` e verificare al login.
- [ ] `.desktop` GUI: una volta creato `speedy-gui.desktop` (vedi
      `todo-fedora.md` §5), copiarlo in `~/.local/share/applications/`
      e verificare che appaia nel menu.

---

## Fedora / Linux packaging — build reale

- [ ] Installare le dipendenze via `dnf` (nomi corretti) e verificare
      `cargo build --release --workspace` verde su Fedora 41+.
      I pacchetti Ubuntu in `ci.yml` sono un'ipotesi — aggiornare il
      job CI Linux con i nomi `dnf` equivalenti se serve una Fedora runner.
- [ ] Icona PNG/SVG per `speedy-gui`: creare e installare in
      `packaging/linux/icons/hicolor/256x256/apps/speedy.png` (e
      scalable). Aggiornare `speedy-gui.desktop` con il path icona
      assoluto o con il nome simbolico `speedy`.
