# TODO — Speedy

Snapshot post-review **2026-05-15** (HEAD `c642282`). Stato:
`cargo check --workspace --all-targets` verde,
`cargo test --workspace -- --test-threads=1` verde su tutti i bin
controllati (speedy-core 78, speedy 15, speedy-cli 39 + e2e 13,
speedy-daemon 65, speedy-core ws-x-proc 2, speedy-mcp 24 + integ 19,
speedy-gui 5).

---

## P0 — Pulizia immediata

- [x] **Rimosso import inutilizzato** `StreamTrait as _` da
      `packages/speedy-cli/src/main.rs`. Zero warning su `cargo check`.

- [x] **`winreg` già assente** da `packages/speedy-gui/Cargo.toml` —
      rimossa prima della review.

- [x] **Fix test harness `speedy-mcp` integration**: `McpClient::stop`
      riscritta con exit write-only + `try_wait` 5 s + `kill()` fallback.
      Risolve il blocco CI Windows.

- [x] **`flow.md` §7 allineato**: aggiunta riga `reembed`.

---

## P1 — Smoke E2E manuale GUI

Richiede macchina fisica. Checklist completa in **[`TODO-platform.md`](./TODO-platform.md)**.

---

## P2 — Fedora / Linux packaging

- [x] §7 `ci.yml`: step `apt-get install` GUI-deps Linux +
      `cargo bench --workspace --no-run`.

Tutto il resto (§1 build Fedora, §3 README Linux, §4 autostart, §5 .desktop)
è già dettagliato in **[`todo-fedora.md`](./todo-fedora.md)**.

---

## P3 — Idee / rifiniture

- [x] **`workspace_status.chunk_count`**: campo già `Option<u64>` con
      doc "None if unknown — kept for forward-compat". Decisione: resta
      `None` dal daemon (aprire SQLite da qui richiederebbe spawning
      aggiuntivo). Nessun cambio necessario.

- [x] **Watcher health test**: `last_heartbeat` / `WATCHER_DEAD_TICKS` /
      `check_watcher_health_with_thresholds` non esistono nel codebase —
      il punto era prematuro. Rimandato a quando la feature verrà
      implementata.

- [x] **`speedy-gui` auto-refresh interval**: aggiunta opzione
      `DragValue` 1–60 s nella sezione Preferenze della Dashboard.
      Persistita via `eframe::Storage` (`refresh_interval_secs`).

- [x] **`speedy-mcp` README**: aggiunto esempio Claude Desktop +
      cross-ref a `commands.md` / `README.md#configuration`.

- [x] **`Justfile` `just build-all`**: aggiunto alias equivalente a
      `cargo build-all`.

- [x] **`scripts/publish.ps1`**: letto — tool release interno one-shot
      (build → crates.io → GitHub Release → bump README URL → tag).
      Non serve sezione "Release" pubblica nel README.

- [x] **Bench + CI**: step `cargo bench --workspace --no-run` in
      `ci.yml`.

---

## Modifiche effettuate (2026-05-15, post-review)

- `packages/speedy-cli/src/main.rs`: rimosso `StreamTrait as _`.
- `packages/speedy-mcp/tests/integration_test.rs`: `McpClient::stop`
  con write-only exit + timeout + kill fallback.
- `flow.md` §7: aggiunta riga `reembed`.
- `.github/workflows/ci.yml`: apt-get GUI-deps Linux + `cargo bench --no-run`.
- `packages/speedy-gui/src/app.rs` + `views/dashboard.rs`:
  `refresh_interval_secs` persistente, DragValue 1–60 s.
- `packages/speedy-mcp/README.md`: esempio Claude Desktop + cross-ref.
- `Justfile`: alias `just build-all`.
