# ───── Speedy – quick commands ─────
# requires `just`: cargo install just

set shell := ["powershell.exe", "-NoProfile", "-Command"]

# default: test + build
default: test build

# test: run all workspace tests
test:
    cargo test --workspace

# test with verbose output
test-verbose:
    cargo test --workspace -- --nocapture

# test only a specific crate
test-crate crate:
    cargo test -p "{{crate}}"

# build: compile everything in debug
build:
    cargo build --workspace

# build-all: alias for cargo build-all (all binaries, release)
build-all:
    cargo build --release -p speedy-ai-context -p speedy-daemon -p speedy-cli -p speedy-mcp -p speedy-gui -p speedy-language-context

# build-slc: only the speedy-language-context binary
build-slc:
    cargo build --release -p speedy-language-context

# build-release: compile optimized
build-release:
    cargo build --release --workspace

# check: analyze without compiling
check:
    cargo check --workspace

# lint: clippy (if installed)
lint:
    cargo clippy --workspace -- -D warnings

# clean: clean everything
clean:
    cargo clean

# run-speedy: run speedy from the workspace
run-speedy cmd *args:
    cargo run -p speedy-ai-context --bin speedy-ai-context -- {{cmd}} {{args}}

# run-cli: run the cli demo
run-cli:
    cargo run -p speedy-ai-context --bin cli

# run-server: run the server demo
run-server:
    cargo run -p speedy-ai-context --bin server

# tree: show the dependency tree
tree:
    cargo tree

# outdated: show outdated dependencies
outdated:
    cargo outdated

# docs: generate documentation
docs:
    cargo doc --workspace --no-deps --open

# fix: fix warnings automatically
fix:
    cargo fix --workspace --allow-dirty

# release <version>: push master + create tag → GitHub Actions builds the exes
# Example: just release 0.2.0
release version:
    powershell -NoProfile -File scripts/publish.ps1 {{version}}

# build-dist: compile all release binaries and copy them to dist\ (no installer).
build-dist:
    powershell -NoProfile -File scripts/build-release.ps1

# dist: release build of all binaries + Inno Setup installer/uninstaller.
# Output: dist\speedy-setup-<version>.exe and dist\speedy-uninstall-<version>.exe.
dist:
    powershell -NoProfile -File scripts/build-installer.ps1

# release-all: dist + kill running Speedy processes + silently install
# the freshly produced .exe into %LOCALAPPDATA%\Programs\Speedy.
# Meant for the local dev loop: recompile, repackage, reinstall in one shot.
release-all: dist
    powershell -NoProfile -File scripts/install-local.ps1
