# Stage 1: Build
FROM rust:slim-trixie AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    cmake \
    g++ \
    git \
    libclang-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install nightly toolchain
RUN rustup toolchain install nightly-2026-02-06 \
    && rustup default nightly-2026-02-06

WORKDIR /build
COPY . .

# Build release binary (all channels by default)
ARG FEATURES="default"
RUN if [ "$FEATURES" = "default" ]; then \
      cargo build --release; \
    else \
      cargo build --release --no-default-features --features "$FEATURES"; \
    fi

# Stage 2: Runtime
FROM debian:trixie-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    ffmpeg \
    libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -s /bin/bash oxicrab

# Copy binary from builder
COPY --from=builder /build/target/release/oxicrab /usr/local/bin/oxicrab

# Create data directory
RUN mkdir -p /home/oxicrab/.oxicrab && chown -R oxicrab:oxicrab /home/oxicrab/.oxicrab

USER oxicrab
WORKDIR /home/oxicrab

# Config and data volume
VOLUME ["/home/oxicrab/.oxicrab"]

# Gateway port (for Twilio webhooks)
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
  CMD pgrep oxicrab || exit 1

ENTRYPOINT ["oxicrab"]
CMD ["gateway"]
