# ---- Stage 1: Builder ----
# Use an official Rust image as the builder.
# Specify a recent stable version. Using slim variant for smaller builder image.
FROM rust:1.86.0-slim as builder
# Check crates.io or rust-lang.org for the latest stable Rust version

# Set the working directory in the container
WORKDIR /usr/src/app

# Install build dependencies if needed (e.g., for C libraries).
# For Debian/Ubuntu based:
# RUN apt-get update && apt-get install -y --no-install-recommends build-essential && rm -rf /var/lib/apt/lists/*
# Our current project likely doesn't need extra deps yet.

# --- Optimized Caching ---
# 1. Copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY api/Cargo.toml ./api/
COPY application/Cargo.toml ./application/
COPY domain/Cargo.toml ./domain/
COPY infrastructure/Cargo.toml ./infrastructure/

# 2. Create dummy source files to build only dependencies
# This allows Docker to cache the dependency layer unless manifests change.
RUN mkdir -p api/src application/src domain/src infrastructure/src && \
    touch api/src/main.rs && \
    touch application/src/lib.rs && \
    touch domain/src/lib.rs && \
    touch infrastructure/src/lib.rs

# 3. Build *only* dependencies
# Target the specific binary we want to produce later ('api').
# Using --locked ensures we use the versions from Cargo.lock.
RUN cargo build --release --locked --bin api

# 4. Remove dummy files after dependency build
RUN rm -rf api/src application/src domain/src infrastructure/src

# 5. Copy the actual source code
COPY . .

# 6. Build the final application binary
# Ensure --locked is used again for reproducible builds.
RUN cargo build --release --locked --bin api

# ---- Stage 2: Runtime ----
# Use a minimal runtime image. debian:slim is a good balance.
# Check Docker Hub for current stable Debian version (e.g., bookworm, bullseye).
FROM debian:bookworm-slim

# Argument for the non-root user
ARG APP_USER=appuser

# Create a non-root user and group for security
RUN groupadd -r ${APP_USER} && \
    useradd --no-log-init -r -g ${APP_USER} ${APP_USER}

# Set the working directory
WORKDIR /app

# Copy the compiled binary from the builder stage
# Renaming the binary for clarity in the runtime image
COPY --from=builder /usr/src/app/target/release/api /app/rust_search_engine

# Ensure the binary is owned by the app user
RUN chown ${APP_USER}:${APP_USER} /app/rust_search_engine

# Switch to the non-root user
USER ${APP_USER}

# Expose the port the application listens on
EXPOSE 3000

# Set environment variables (defaults can be overridden in docker-compose or via `docker run -e`)
ENV RUST_LOG="info,api=info"
ENV RUST_BACKTRACE=0

# Define the command to run the application
CMD ["./rust_search_engine"]