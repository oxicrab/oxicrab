# Stage 1: Chef — prepare dependency recipe
FROM rust:slim-trixie AS chef

RUN apt-get update && apt-get install -y \
    cmake \
    g++ \
    git \
    libclang-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Read nightly version from rust-toolchain.toml (single source of truth)
COPY rust-toolchain.toml .
RUN NIGHTLY=$(grep 'channel' rust-toolchain.toml | sed 's/.*"\(.*\)"/\1/') \
    && rustup toolchain install "$NIGHTLY" \
    && rustup default "$NIGHTLY"

RUN cargo install cargo-chef --locked

WORKDIR /build

# Stage 2: Planner — compute dependency recipe from source
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder — build dependencies first (cached), then source
FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json

# Build dependencies only (this layer is cached unless Cargo.toml/lock change)
ARG FEATURES="default"
RUN if [ "$FEATURES" = "default" ]; then \
      cargo chef cook --release --recipe-path recipe.json; \
    else \
      cargo chef cook --release --no-default-features --features "$FEATURES" --recipe-path recipe.json; \
    fi

# Now copy source and build (only recompiles oxicrab, not deps)
COPY . .
RUN if [ "$FEATURES" = "default" ]; then \
      cargo build --release; \
    else \
      cargo build --release --no-default-features --features "$FEATURES"; \
    fi

# Stage 4: Runtime
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    ffmpeg \
    libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash oxicrab

COPY --from=builder /build/target/release/oxicrab /usr/local/bin/oxicrab

COPY scripts/healthcheck.sh /usr/local/bin/healthcheck.sh
RUN chmod +x /usr/local/bin/healthcheck.sh

RUN mkdir -p /home/oxicrab/.oxicrab && chown -R oxicrab:oxicrab /home/oxicrab/.oxicrab

USER oxicrab
WORKDIR /home/oxicrab

VOLUME ["/home/oxicrab/.oxicrab"]

EXPOSE 18790

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD /usr/local/bin/healthcheck.sh

ENTRYPOINT ["oxicrab"]
CMD ["gateway"]
