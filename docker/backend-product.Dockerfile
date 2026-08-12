FROM rust:1.87-slim AS builder
ARG PRODUCT_SLUG
ARG BINARY_NAME
WORKDIR /app
RUN apt-get update \
    && apt-get install -y pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
COPY crates ./crates
COPY Cargo.lock /tmp/patchhive-Cargo.lock
COPY services/patchhive-backend/registry ./services/patchhive-backend/registry
COPY products/${PRODUCT_SLUG}/backend ./products/${PRODUCT_SLUG}/backend
RUN cp /tmp/patchhive-Cargo.lock products/${PRODUCT_SLUG}/backend/Cargo.lock \
    && cargo build --release --locked --manifest-path products/${PRODUCT_SLUG}/backend/Cargo.toml \
    && cp products/${PRODUCT_SLUG}/backend/target/release/${BINARY_NAME} /tmp/patchhive-product

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 patchhive \
    && mkdir -p /app \
    && chown patchhive:patchhive /app
COPY --from=builder /tmp/patchhive-product /usr/local/bin/patchhive-product
WORKDIR /app
USER patchhive
CMD ["patchhive-product"]
