# LINUX-12: Container-native deployment for Mivon HDL Simulator
# Multi-stage build: build + runtime

# Stage 1: Build
FROM rust:1.78-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

RUN cargo build --release --bin mivon

# Stage 2: Runtime
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r mivon && useradd -r -g mivon -s /bin/bash mivon

WORKDIR /home/mivon

# Copy binary from builder
COPY --from=builder /app/target/release/mivon /usr/local/bin/mivon

# Copy UVM macros
COPY --from=builder /app/uvm_macros.svh /home/mivon/uvm_macros.svh

# Set ownership
RUN chown -R mivon:mivon /home/mivon

USER mivon

# Default: show help
CMD ["mivon", "--help"]

# Labels
LABEL org.opencontainers.image.title="Mivon HDL Simulator"
LABEL org.opencontainers.image.description="Rust-based SystemVerilog RTL simulator"
LABEL org.opencontainers.image.version="0.3.0"
LABEL org.opencontainers.image.source="https://github.com/mivonsim/mivon"
