FROM rust:1.69-bullseye

RUN rustup component add clippy rustfmt
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo install cargo-deny && rm -rf /root/.cargo/registry

RUN apt-get update && apt-get install -y --no-install-recommends \
    libclang-dev llvm-dev libkmod-dev && \
    rm -rf /var/lib/apt/lists/*
