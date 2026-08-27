# Multi-stage build: compile aikv with the GitHub main aidb dependency, ship minimal runtime.
# rust:bookworm is the available official Bookworm image and ships stable Rust.
FROM rust:bookworm@sha256:e536cf316987faedfe8ae120f83b70c7df0068fdb4fc9efcce55c71a625001d5 AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src/aikv

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src

RUN cargo build --locked --release --features cluster,monitoring,compression

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/aikv/target/release/aikv /usr/local/bin/aikv

EXPOSE 6379 16379 26379 9191

ENTRYPOINT ["/usr/local/bin/aikv"]
