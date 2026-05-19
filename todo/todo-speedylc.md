# speedy-language-context — TODO (aperto)

Binario funzionante. Lingue attive: Rust, TypeScript, JavaScript, Python, Go, JSX, TSX,
C, C++, Java, C#, Ruby, Swift, Scala, PHP.
Workspace compila clean, tutti i test passano.

---

## Linguaggi da aggiungere (FASE 6)

- [ ] **Kotlin** — bloccato da incompatibilità ABI di `tree-sitter-kotlin`.

  **Problema tecnico**: Speedy usa `tree-sitter` 0.25 (grammar ABI 15). Il crate
  `tree-sitter-kotlin` 0.3.x è compilato contro tree-sitter 0.20 (ABI 13) — versioni
  diverse della C ABI non sono linkabili e causano errori a compile-time o panic a runtime.

  **Stato attuale**: `.kt` / `.kts` sono silenziosamente skippati in
  `packages/speedy-language-context/src/parser/tree_sitter_parser.rs` (nessun crash,
  ma nessun simbolo Kotlin viene indicizzato).

  **Come sbloccare**: monitorare https://crates.io/crates/tree-sitter-kotlin fino a quando
  appare un release compatibile con tree-sitter ≥ 0.23 (ABI 14+).
  Quando disponibile:
  1. Aggiungere `tree-sitter-kotlin = "x.y"` a `packages/speedy-language-context/Cargo.toml`
  2. Rimuovere il commento di skip in `tree_sitter_parser.rs` e aggiungere il caso
     `"kt" | "kts" => Some(tree_sitter_kotlin::language())` alla match degli extension

Aggiornato `tree-sitter` da 0.22 → 0.25 (necessario per grammar ABI 15 usato dai crate 0.23+).
`packages/speedy-language-context/Cargo.toml`, `src/parser/tree_sitter_parser.rs`.
