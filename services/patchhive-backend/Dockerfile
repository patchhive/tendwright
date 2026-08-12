# syntax=docker/dockerfile:1.7

ARG RUST_IMAGE=rust:1.97.1-bookworm@sha256:77fac8b98f9f46062bb680b6d25d5bcaabfc400143952ebc572e924bcbedc3fa
ARG DOCKER_CLI_IMAGE=docker:29.6.2-cli@sha256:be132a9f282288de4afaf63379dff75711fda0147c6b72a9df44e51841402144
ARG RUNTIME_IMAGE=debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818

FROM ${DOCKER_CLI_IMAGE} AS docker-cli

FROM ${RUST_IMAGE} AS builder

WORKDIR /workspace

# The unified manifest uses path dependencies across the monorepo. Keep the
# build context at the repository root and copy only Rust inputs.
COPY Cargo.toml Cargo.lock ./
COPY crates/patchhive-product-core ./crates/patchhive-product-core
COPY crates/patchhive-github-data ./crates/patchhive-github-data
COPY crates/patchhive-github-pr ./crates/patchhive-github-pr
COPY crates/patchhive-github-security ./crates/patchhive-github-security
COPY packages/ai-local/rust-gateway ./packages/ai-local/rust-gateway
COPY products/dep-triage/backend ./products/dep-triage/backend
COPY products/flake-sting/backend ./products/flake-sting/backend
COPY products/hive-core/backend ./products/hive-core/backend
COPY products/merge-keeper/backend ./products/merge-keeper/backend
COPY products/refactor-scout/backend ./products/refactor-scout/backend
COPY products/release-sentry/backend ./products/release-sentry/backend
COPY products/repo-memory/backend ./products/repo-memory/backend
COPY products/repo-reaper/backend ./products/repo-reaper/backend
COPY products/review-bee/backend ./products/review-bee/backend
COPY products/signal-hive/backend ./products/signal-hive/backend
COPY products/trust-gate/backend ./products/trust-gate/backend
COPY products/vuln-triage/backend ./products/vuln-triage/backend
COPY services/patchhive-backend ./services/patchhive-backend

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/workspace/target,sharing=locked \
    CARGO_TARGET_DIR=/workspace/target cargo build \
      --release \
      --locked \
      --bins \
      --manifest-path services/patchhive-backend/Cargo.toml \
    && install -D /workspace/target/release/patchhive-backend /workspace/out/patchhive-backend \
    && install -D /workspace/target/release/consolidate-databases /workspace/out/consolidate-databases

FROM ${RUNTIME_IMAGE} AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 patchhive \
    && useradd --uid 10001 --gid patchhive --create-home --shell /usr/sbin/nologin patchhive \
    && install -d --owner patchhive --group patchhive /var/lib/patchhive

COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=builder /workspace/out/patchhive-backend /usr/local/bin/patchhive-backend
COPY --from=builder /workspace/out/consolidate-databases /usr/local/bin/consolidate-databases

ENV PATCHHIVE_BIND_ADDR=0.0.0.0:8100 \
    PATCHHIVE_DB_PATH=/var/lib/patchhive/patchhive.db \
    PATCHHIVE_PRODUCTS=all

WORKDIR /var/lib/patchhive
USER 10001:10001

VOLUME ["/var/lib/patchhive"]
EXPOSE 8100
STOPSIGNAL SIGTERM

ENTRYPOINT ["patchhive-backend"]
