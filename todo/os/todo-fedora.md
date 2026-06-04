# Speedy on Fedora — TODO

Bring the same "build + install + autostart" flow that is currently documented
only for Windows to Fedora as well (and, by extension, to generic Linux). The
workspace already compiles (unix target, no Windows-only APIs in the active
paths — `winreg`/`uds_windows` are behind `cfg(windows)`), so the work is
**packaging, documentation and desktop integration**, not new feature code.

---

## 1. Verify the actual build on Fedora

Before promising "compiles everywhere" we need to actually prove it.

- [ ] Clean build on Fedora 41+ with stable `rustup`:
      ```
      cargo build --release --workspace
      ```
      Note down the `dnf` packages that are really needed (below is a plausible
      list, it needs to be validated).
- [ ] `cargo test --workspace` green. Watch out for the daemon tests that
      serialize on a global mutex (they might require
      `-- --test-threads=1`).
- [ ] Interactive launch `cargo run --release -p speedy-gui` on:
  - [ ] GNOME / Wayland (tray icon requires the AppIndicator extension).
  - [ ] KDE Plasma / Wayland (native tray).
  - [ ] X11 fallback.
- [ ] Verify that `interprocess` on Linux uses UDS in
      `$XDG_RUNTIME_DIR/speedy-daemon` (or equivalent) and that the path is
      stable across logout/login.

### Candidate `dnf` packages

```
gcc pkgconf-pkg-config
glib2-devel gtk3-devel
libxkbcommon-devel libxcb-devel wayland-devel
libappindicator-gtk3-devel        # needed for tray-icon
openssl-devel
```

`rusqlite` has `features = ["bundled"]` → no `sqlite-devel`.
Does `reqwest` use rustls by default? Check `Cargo.lock` to figure out whether
`openssl-devel` is really needed (if it's rustls only, it can be removed).

---

## 2. Existing Linux build script

`scripts/build-release.sh` exists and copies into `dist/`. To be refined:

- [ ] Add an upfront check of the system dependencies (a `pkg-config
      --exists gtk+-3.0 ayatana-appindicator3-0.1` with a clear message if
      they are missing), so the error isn't a wall of text from `cargo`.
- [ ] Print the list of the 5 produced binaries with their sizes (cosmetic).

---

## 3. README documentation — Linux section

Today the README only has the Windows section ("Recommended layout (Windows)",
"Startup folder"). Add a twin paragraph.

- [ ] **Recommended install path**: `~/.local/bin/` (already on `PATH` by
      default on Fedora) for the 4 front-end binaries (`speedy`, `speedy-cli`,
      `speedy-mcp`, `speedy-gui`). Keep `speedy-daemon` separate (see
      autostart below).
- [ ] **Copy command** equivalent to the Windows PowerShell block:
      ```
      install -Dm755 dist/speedy        ~/.local/bin/speedy
      install -Dm755 dist/speedy-cli    ~/.local/bin/speedy-cli
      install -Dm755 dist/speedy-mcp    ~/.local/bin/speedy-mcp
      install -Dm755 dist/speedy-gui    ~/.local/bin/speedy-gui
      install -Dm755 dist/speedy-daemon ~/.local/libexec/speedy-daemon
      ```
- [ ] "**Daemon autostart**" section with the two approaches (see §4).
- [ ] "**Tray icon on GNOME**" section explaining that the extension
      *AppIndicator and KStatusNotifierItem Support* (`gnome-extensions`) must
      be installed, otherwise the tray won't appear. On KDE/Cinnamon/XFCE it works out-of-the-box.
- [ ] Update the binaries table to drop the `.exe` suffix
      when talking about Linux, or make two separate tables.
- [ ] Config path: it already mentions `~/.config/speedy/` but only in passing,
      promote it to a paragraph in the Linux section.

---

## 4. Daemon autostart on Linux

The equivalent of the Windows Startup folder. Two options, both
documented, the user chooses.

### 4.1 Option A — systemd user service (recommended)

- [ ] Write `packaging/linux/speedy-daemon.service`:
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
- [ ] Document in the README:
      ```
      mkdir -p ~/.config/systemd/user
      cp packaging/linux/speedy-daemon.service ~/.config/systemd/user/
      systemctl --user daemon-reload
      systemctl --user enable --now speedy-daemon
      ```
- [ ] `loginctl enable-linger $USER` (optional, if the user wants the
      daemon active even without an open graphical session — useful on servers,
      probably NOT to be recommended by default on desktop).

### 4.2 Option B — XDG autostart (.desktop)

Simpler, starts only when the user logs in graphically.

- [ ] Write `packaging/linux/speedy-daemon.desktop`:
      ```
      [Desktop Entry]
      Type=Application
      Name=Speedy Daemon
      Exec=%h/.local/libexec/speedy-daemon
      X-GNOME-Autostart-enabled=true
      NoDisplay=true
      ```
- [ ] Document the path: `~/.config/autostart/speedy-daemon.desktop`.

---

## 5. Desktop integration

- [ ] **`.desktop` for the GUI**: `packaging/linux/speedy-gui.desktop` with
      `Icon=speedy`, `Categories=Development;Utility;` so it appears in the
      applications menu on GNOME/KDE.
- [ ] **Icon**: a PNG (at least 256x256) or SVG is needed. Today the tray uses
      an icon generated in code — that's fine for the tray, but for the
      `.desktop` entry a file installed in
      `~/.local/share/icons/hicolor/256x256/apps/speedy.png` (or its
      system-wide equivalent) is needed.
- [ ] MIME type for workspaces? Probably not — we don't open files
      directly. Skip unless requested.

---

## 6. Real packaging (deferred, optional)

To be decided whether it's worth the effort or whether "download the binaries
and copy them into `~/.local/bin/`" is enough for now.

- [ ] **RPM**: spec file in `packaging/rpm/speedy.spec`, build with
      `rpmbuild` or `cargo-generate-rpm`. Advantage: it installs everything in
      `/usr/bin`, `/usr/libexec`, `/usr/share/applications` and handles the
      dependencies (`Requires: ollama` — even though Ollama isn't always in
      the official Fedora repos).
- [ ] **COPR**: once the spec works, publish on
      `copr.fedorainfracloud.org` for a direct `dnf install speedy`.
- [ ] **AppImage**: alternative to RPM, single-file, runs even outside Fedora.
      Probably overkill — a tarball of static binaries is equivalent.

---

## 7. CI

Current state (verified 2026-05-15):

- [x] **`ci.yml`** already runs the matrix `[ubuntu-latest, macos-latest, windows-latest]`
      on `cargo build` + `cargo test --workspace`. The Linux job is
      effectively close to Fedora for compilation, that's enough. It does
      however need to be **integrated** with `apt-get install` of the GUI
      dependencies (libgtk-3-dev, libxkbcommon-dev, etc.) — today `ci.yml`
      does not install them, so if in the future compiling `speedy-gui`
      were to require them directly in unit tests, the job would fail.
      `release.yml` already does this correctly.
- [x] **`release.yml`** builds the 5 binaries on `x86_64-unknown-linux-gnu`,
      `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`
      and produces tarballs — it already includes `apt-get` for the Linux deps.
- [ ] Align `ci.yml` with `release.yml` on Linux: add the
      `Install Linux GUI dependencies` step to the CI job too, so
      `speedy-gui` is actually built and tested.
- [ ] Possibly a dedicated `fedora:latest` job in a container
      (`container: fedora:41`) for exact assurance on the `dnf` packages.
      Slower, evaluate only if deb↔rpm divergences emerge.

---

## 8. Quick references

- Cargo workspace root: `Cargo.toml`
- Current build script: `scripts/build-release.sh`
- Config path on Linux: `~/.config/speedy/` (`workspaces.json`, `daemon.pid`)
- UDS socket: managed by `interprocess` in `packages/speedy-core/`
- IPC documentation: `docs/ipc-protocol.md`
