# PipeDeck dev/build container — mirrors chronos (Ubuntu 26.04, PipeWire 1.6, Rust 1.93).
# Compile + unit tests only; there is no PipeWire runtime inside. Integration runs on chronos.
FROM ubuntu:26.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      cargo rustc rust-clippy rustfmt build-essential pkg-config clang libclang-dev \
      libpipewire-0.3-dev libdbus-1-dev gjs ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
ENV CARGO_HOME=/cargo
WORKDIR /src
