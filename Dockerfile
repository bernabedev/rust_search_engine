# ---- Stage 1: Builder ----
FROM rust:1.86.0-slim as builder
WORKDIR /usr/src/app

# Install build dependencies if needed (uncomment if necessary)
# RUN apt-get update && apt-get install -y --no-install-recommends build-essential && rm -rf /var/lib/apt/lists/*

# --- Simplified Caching ---
# 1. Copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY api/Cargo.toml ./api/
COPY application/Cargo.toml ./application/
COPY domain/Cargo.toml ./domain/
COPY infrastructure/Cargo.toml ./infrastructure/

# 2. Copy all source code next
COPY api ./api/
COPY application ./application/
COPY domain ./domain/
COPY infrastructure ./infrastructure/

# 3. Build the final application binary directly
# This will build dependencies the first time and reuse the layer if only code changes afterwards
# (though changes in any crate might trigger a rebuild from here)
RUN cargo build --release --locked --bin api

# ---- Stage 2: Runtime ----
# (This stage remains the same as before)
FROM debian:bookworm-slim

ARG APP_USER=appuser
RUN sh -c 'groupadd -r ${APP_USER} && useradd --no-log-init -r -g ${APP_USER} ${APP_USER}'

WORKDIR /app

COPY --from=builder /usr/src/app/target/release/api /app/rust_search_engine
RUN chown ${APP_USER}:${APP_USER} /app/rust_search_engine

USER ${APP_USER}

EXPOSE 3000
ENV RUST_LOG="info,api=info"
ENV RUST_BACKTRACE=0

CMD ["./rust_search_engine"]