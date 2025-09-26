# Multi-stage build for optimized production image
FROM rust:1.70-slim as builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /usr/src/app

# Copy Cargo files
COPY Cargo.toml Cargo.lock ./
COPY telemetry-core/Cargo.toml ./telemetry-core/
COPY telemetry-ingestion/Cargo.toml ./telemetry-ingestion/
COPY telemetry-analysis/Cargo.toml ./telemetry-analysis/
COPY telemetry-ml/Cargo.toml ./telemetry-ml/
COPY telemetry-alerts/Cargo.toml ./telemetry-alerts/
COPY telemetry-can/Cargo.toml ./telemetry-can/

# Build dependencies (this layer will be cached)
RUN mkdir -p telemetry-core/src telemetry-ingestion/src telemetry-analysis/src \
    telemetry-ml/src telemetry-alerts/src telemetry-can/src src && \
    echo "fn main() {}" > src/main.rs && \
    echo "// dummy" > telemetry-core/src/lib.rs && \
    echo "// dummy" > telemetry-ingestion/src/lib.rs && \
    echo "// dummy" > telemetry-analysis/src/lib.rs && \
    echo "// dummy" > telemetry-ml/src/lib.rs && \
    echo "// dummy" > telemetry-alerts/src/lib.rs && \
    echo "// dummy" > telemetry-can/src/lib.rs && \
    cargo build --release && \
    rm -rf src telemetry-*/src

# Copy source code
COPY . .

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bullseye-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    can-utils \
    iproute2 \
    && rm -rf /var/lib/apt/lists/*

# Create app user
RUN useradd -r -s /bin/false telemetry

# Create directories
RUN mkdir -p /app/config /app/data /app/logs && \
    chown -R telemetry:telemetry /app

# Copy binary
COPY --from=builder /usr/src/app/target/release/telemetry-system /app/
COPY --from=builder /usr/src/app/config/ /app/config/

# Set permissions
RUN chmod +x /app/telemetry-system

# Switch to app user
USER telemetry

# Set working directory
WORKDIR /app

# Expose ports
EXPOSE 8080 8888

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Run the application
CMD ["./telemetry-system"]
