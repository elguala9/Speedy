# ───── Speedy – comandi rapidi ─────
# richiede `just`: cargo install just

set shell := ["powershell.exe", "-NoProfile", "-Command"]

# default: test + build
default: test build

# test: esegue tutti i test del workspace
test:
    cargo test --workspace

# test con output verboso
test-verbose:
    cargo test --workspace -- --nocapture

# test solo di un crate specifico
test-crate crate:
    cargo test -p "{{crate}}"

# build: compila tutto in debug
build:
    cargo build --workspace

# build-release: compila ottimizzato
build-release:
    cargo build --release --workspace

# check: analisi senza compilare
check:
    cargo check --workspace

# lint: clippy (se installato)
lint:
    cargo clippy --workspace -- -D warnings

# clean: pulisce tutto
clean:
    cargo clean

# run-speedy: esegue speedy dal workspace
run-speedy cmd *args:
    cargo run -p speedy --bin speedy -- {{cmd}} {{args}}

# run-cli: esegue il cli demo
run-cli:
    cargo run -p speedy --bin cli

# run-server: esegue il server demo
run-server:
    cargo run -p speedy --bin server

# tree: mostra l'albero dipendenze
tree:
    cargo tree

# outdated: mostra dipendenze outdated
outdated:
    cargo outdated

# docs: genera documentazione
docs:
    cargo doc --workspace --no-deps --open

# fix: corregge warning automaticamente
fix:
    cargo fix --workspace --allow-dirty

# dist: build release dei 3 binari principali e copia in dist/
dist:
    powershell -NoProfile -File scripts/build-release.ps1

# publish: build release, publish su crates.io, crea GitHub Release con .exe, aggiorna README
publish:
    powershell -NoProfile -File scripts/publish.ps1 
