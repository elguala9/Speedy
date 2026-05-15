# Multi-stage build: compile Rust binary, copy into minimal runtime image.

# ── Stage 1: builder ─────────────────────────────────────────────────────────
FROM rust:1.78-slim AS builder

WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
        musl-tools pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-musl

# Cache dependency compilation separately from source
COPY Cargo.toml Cargo.lock ./
COPY packages/ ./packages/
RUN cargo build --release --target x86_64-unknown-linux-musl \
    --bin speedy-service \
    && strip target/x86_64-unknown-linux-musl/release/speedy-service

# ── Stage 2: runtime ─────────────────────────────────────────────────────────
FROM scratch

LABEL org.opencontainers.image.source="https://github.com/example/speedy"
LABEL org.opencontainers.image.licenses="MIT"

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder \
    /build/target/x86_64-unknown-linux-musl/release/speedy-service \
    /usr/local/bin/speedy-service

ENV SPEEDY_MODEL=all-minilm:l6-v2
ENV SPEEDY_LOG=info

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/speedy-service"]
