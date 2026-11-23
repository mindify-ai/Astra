# astra-server container image.
# Build from the repository root:  docker build -f crates/astra-server/Dockerfile -t astra-server .
# Runtime config is environment-only (PORT, DATABASE_URL, ASTRA_JWT_SECRET, ...; see crates/astra-server/src/lib.rs).

FROM rust:1-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY fixtures/ fixtures/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p astra-server \
    && cp target/release/astra-server /usr/local/bin/astra-server

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /usr/local/bin/astra-server /usr/local/bin/astra-server
ENV PORT=8080
EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/astra-server"]
