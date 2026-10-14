# Build stage
FROM rust:1.80-slim-bookworm AS builder

WORKDIR /usr/src/spoor

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary
RUN cargo build --release --bin spoor

# Runtime stage
FROM debian:bookworm-slim

# Install ca-certificates for HTTPS calls
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r spoor && useradd -r -g spoor -d /home/spoor -m -s /bin/bash spoor

# Copy binary from builder
COPY --from=builder /usr/src/spoor/target/release/spoor /usr/local/bin/spoor

# Prepare cache directory
RUN mkdir -p /var/lib/spoor && chown -R spoor:spoor /var/lib/spoor
ENV SPOOR_CACHE_PATH=/var/lib/spoor/cache.sqlite

USER spoor
WORKDIR /home/spoor

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD curl -f http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["spoor"]
CMD ["serve", "--bind", "0.0.0.0", "--port", "8080"]
