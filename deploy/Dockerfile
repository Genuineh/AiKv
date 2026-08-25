# Multi-stage build: compile aikv with cloud AiDb dependency, ship minimal runtime.
# rust:bookworm is the available official Bookworm image and ships stable Rust.
FROM rust:bookworm@sha256:e536cf316987faedfe8ae120f83b70c7df0068fdb4fc9efcce55c71a625001d5 AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        git \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src/aikv

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src

# The host lockfile records aidb as a path package when the developer's
# ~/.cargo/config.toml applies the local sibling [patch]. Rewrite only that
# package's source in the image, then refresh the Git branch dependency while
# retaining the lockfile's registry resolutions. If a future lockfile already
# has a source, leave it unchanged; reject ambiguous multiple path entries.
RUN awk '\
function flush_package() { \
    if (package_is_aidb && !package_has_source) { \
        missing_aidb++; \
        print "source = \"git+https://github.com/wiqun/AiDb.git?branch=new%2Fmain#db0d34ab0e7542ccac95c5eafa1ddcf961e01007\""; \
    } \
} \
/^\[\[package\]\]$/ { flush_package(); package_is_aidb = 0; package_has_source = 0; print; next } \
/^name = "aidb"$/ { package_is_aidb = 1 } \
package_is_aidb && /^source = / { package_has_source = 1 } \
{ print } \
END { flush_package(); if (missing_aidb > 1) exit 1 }' \
    Cargo.lock > Cargo.lock.tmp \
    && mv Cargo.lock.tmp Cargo.lock \
    && cargo update -p aidb \
    && cargo build --release --features cluster,monitoring,compression

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/aikv/target/release/aikv /usr/local/bin/aikv

EXPOSE 6379 16379 26379 9191

ENTRYPOINT ["/usr/local/bin/aikv"]
