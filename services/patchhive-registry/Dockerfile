ARG RUST_IMAGE=rust:1.97.1-bookworm@sha256:77fac8b98f9f46062bb680b6d25d5bcaabfc400143952ebc572e924bcbedc3fa

FROM ${RUST_IMAGE} AS build
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
