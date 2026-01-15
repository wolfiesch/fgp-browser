# FGP Browser Daemon Docker Image
#
# Provides fast browser automation via Chrome DevTools Protocol.
# Uses multi-stage build for minimal image size.

# Stage 1: Build the Rust binary
FROM rust:slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Create dummy src to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src target/release/browser-gateway

# Copy actual source and build
COPY src ./src
RUN touch src/main.rs && cargo build --release

# Stage 2: Runtime image with Chrome
FROM debian:bookworm-slim

# Install Chrome and runtime dependencies
RUN apt-get update && apt-get install -y \
    chromium \
    chromium-sandbox \
    ca-certificates \
    fonts-liberation \
    libnss3 \
    libxss1 \
    libasound2 \
    libatk-bridge2.0-0 \
    libgtk-3-0 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -s /bin/bash fgp

# Copy binary from builder
COPY --from=builder /app/target/release/browser-gateway /usr/local/bin/

# Set up FGP directory structure
RUN mkdir -p /home/fgp/.fgp/services/browser/logs \
    && chown -R fgp:fgp /home/fgp/.fgp

USER fgp
WORKDIR /home/fgp

# Set Chrome path for the daemon
ENV CHROME_PATH=/usr/bin/chromium
ENV FGP_SOCKET_DIR=/home/fgp/.fgp/services

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD browser-gateway status || exit 1

# Expose the socket via volume mount (UNIX sockets can't be exposed as ports)
VOLUME ["/home/fgp/.fgp/services"]

ENTRYPOINT ["browser-gateway"]
CMD ["start", "--foreground"]
