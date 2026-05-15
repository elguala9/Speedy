# TODO — Speedy

Snapshot post-review **2026-05-15** (HEAD `c642282`). Stato:
`cargo check --workspace --all-targets` verde,
`cargo test --workspace -- --test-threads=1` verde su tutti i bin
controllati (speedy-core 78, speedy 15, speedy-cli 39 + e2e 13,
speedy-daemon 65, speedy-core ws-x-proc 2, speedy-mcp 24 + integ 19,
speedy-gui 5).

---

## P1 — Smoke E2E manuale GUI

Richiede macchina fisica. Checklist completa in **[`TODO-platform.md`](./TODO-platform.md)**.

---

## P2 — Fedora / Linux packaging

Tutto il resto (§1 build Fedora, §3 README Linux, §4 autostart, §5 .desktop)
è già dettagliato in **[`todo-fedora.md`](./todo-fedora.md)**.

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
