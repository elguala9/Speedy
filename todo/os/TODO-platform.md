# TODO — Platform / Hardware

Items that require a physical machine or a specific OS to be verified.
They don't block the build/CI; they must be done manually on each target platform.

---

## GUI smoke E2E — Windows

`cargo run --release -p speedy-gui` with a live daemon. Never run
interactively since part 3 of 2026-05-15 onward.

- [ ] Tray icon appears green; right click → Open / Restart / Quit OK.
- [ ] Dashboard: PID/uptime/version visible; metrics update
      after a `speedy-cli index` on a test workspace.
- [ ] Auto-refresh interval DragValue (1–60 s) actually changes
      the poll frequency and persists after a GUI restart.
- [ ] "Daemon executable" override: `Browse…` opens a file picker;
      `Apply` updates the path; `Reset to automatic` returns
      to auto-detect; persistence after a GUI restart.
- [ ] Workspaces: add via picker; "Clean orphans" removes workspaces
      whose path has been deleted; "⚠ temp" / "⚠ missing" badges
      consistent.
- [ ] Logs: `subscribe-log` stream receives events; "Historical file" switch
      loads `daemon.log.YYYY-MM-DD`; `.json` / `.jsonl` export opens a
      save dialog.
- [ ] System notifications on `error`: `RUST_LOG=error cargo run …`
      triggers a system notification.

---

## GUI smoke E2E — macOS

Same round as the Windows section. Never tested.

- [ ] Build: `cargo build --release -p speedy-gui` (deps: `brew install`
      if libxkbcommon or similar is needed — verify).
- [ ] Tray icon appears (macOS uses `NSStatusItem`; `tray-icon` 0.19
      should support it — to be confirmed).
- [ ] All the Windows checklist points repeated on macOS.
- [ ] Autostart: shortcut in `~/Library/LaunchAgents/` or
      `Login Items` — document the recommended method.

---

## GUI smoke E2E — Linux GNOME / KDE

See also `todo-fedora.md` §1.

- [ ] Build on Fedora 41+: confirm that the `dnf` packages installed
      in CI (`apt-get` is Ubuntu — on Fedora different names are needed, e.g.
      `gtk3-devel`, `glib2-devel` …).
- [ ] `cargo build --release -p speedy-gui` green on Fedora 41.
- [ ] Tray icon on GNOME: requires the AppIndicator extension or
      `libayatana-appindicator` — verify whether `tray-icon` 0.19 uses
      it or whether a workaround is needed.
- [ ] Tray icon on KDE Plasma: should work out-of-the-box.
- [ ] File picker (`rfd` 0.14) via GTK — test.
- [ ] Notifications (`notify-rust` 4.11) via D-Bus — test.
- [ ] All the Windows checklist points repeated.
- [ ] Autostart: once `speedy-daemon.service` is created (see
      `todo-fedora.md` §4), enable it with
      `systemctl --user enable --now speedy-daemon` and verify at login.
- [ ] `.desktop` GUI: once `speedy-gui.desktop` is created (see
      `todo-fedora.md` §5), copy it into `~/.local/share/applications/`
      and verify that it appears in the menu.

---

## Fedora / Linux packaging — real build

- [ ] Install the dependencies via `dnf` (correct names) and verify
      `cargo build --release --workspace` is green on Fedora 41+.
      The Ubuntu packages in `ci.yml` are a guess — update the
      Linux CI job with the equivalent `dnf` names if a Fedora runner is needed.
- [ ] PNG/SVG icon for `speedy-gui`: create and install in
      `packaging/linux/icons/hicolor/256x256/apps/speedy.png` (and
      scalable). Update `speedy-gui.desktop` with the absolute icon path
      or with the symbolic name `speedy`.
