# LINUX-12: Container-native deployment for Maria HDL Simulator
# Multi-stage build: build + runtime

# Stage 1: Build
FROM rust:1.78-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

RUN cargo build --release --bin maria

# Stage 2: Runtime
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r maria && useradd -r -g maria -s /bin/bash maria

WORKDIR /home/maria

# Copy binary from builder
COPY --from=builder /app/target/release/maria /usr/local/bin/maria

# Copy UVM macros
COPY --from=builder /app/uvm_macros.svh /home/maria/uvm_macros.svh

# Set ownership
RUN chown -R maria:maria /home/maria

USER maria

# Default: show help
CMD ["maria", "--help"]

# Labels
LABEL org.opencontainers.image.title="Maria HDL Simulator"
LABEL org.opencontainers.image.description="Rust-based SystemVerilog RTL simulator"
LABEL org.opencontainers.image.version="0.3.0"
LABEL org.opencontainers.image.source="https://github.com/Yoriyoi-drop/maria"
