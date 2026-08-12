FROM rust:1.85-bookworm AS build
WORKDIR /app
COPY crates /app/crates
COPY Cargo.lock /app/services/patchhive-registry/Cargo.lock
COPY services/patchhive-registry /app/services/patchhive-registry
WORKDIR /app/services/patchhive-registry
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd -r -u 10001 patchhive
WORKDIR /app
COPY --from=build /app/services/patchhive-registry/target/release/patchhive-registry /usr/local/bin/patchhive-registry
USER patchhive
ENV PATCHHIVE_REGISTRY_BIND_ADDR=0.0.0.0:8130
EXPOSE 8130
CMD ["patchhive-registry"]
