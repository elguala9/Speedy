# speedy-language-context — TODO (aperto)

Binario funzionante. Lingue attive: Rust, TypeScript, JavaScript, Python, Go, JSX, TSX,
C, C++, Java, C#, Ruby, Swift, Scala, PHP.
Workspace compila clean, tutti i test passano.

---

## Linguaggi da aggiungere (FASE 6)

- [ ] **Kotlin** — `tree-sitter-kotlin` 0.3.x usa tree-sitter 0.20, incompatibile con 0.25. Da aggiungere quando il crate verrà aggiornato.

Aggiornato `tree-sitter` da 0.22 → 0.25 (necessario per grammar ABI 15 usato dai crate 0.23+).
`packages/speedy-language-context/Cargo.toml`, `src/parser/tree_sitter_parser.rs`.
