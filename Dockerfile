# syntax=docker/dockerfile:1
# Optional container image (spec §7.6). The preferred deployment is the single
# `paycheckzero` binary plus a SQLite file; this image wraps exactly that.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
# Cache mounts keep downloaded crates and compiled output between builds, so
# a code change recompiles only this workspace, not every dependency. The
# `docker` profile trades a little binary size for a much faster, lighter
# build (thin LTO, parallel codegen); see Cargo.toml. Set CARGO_BUILD_JOBS
# (e.g. --build-arg CARGO_BUILD_JOBS=1) on a machine short of memory.
ARG CARGO_BUILD_JOBS
RUN --mount=type=cache,id=pz-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=pz-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=pz-target,target=/src/target \
    cargo build --profile docker -p paycheckzero-web --bin paycheckzero \
    && cp target/docker/paycheckzero /usr/local/bin/paycheckzero

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /data paycheckzero && mkdir -p /data && chown paycheckzero /data
COPY --from=build /usr/local/bin/paycheckzero /usr/local/bin/paycheckzero
USER paycheckzero
# /data must be the working directory: PZ_DATA_KEY_FILE defaults to the relative
# path "paycheckzero.key", and anything written outside the volume is lost on
# container replacement (bank tokens stop decrypting).
WORKDIR /data
ENV PZ_BIND=0.0.0.0:8080 \
    PZ_DATABASE_URL=sqlite:///data/paycheckzero.db?mode=rwc \
    PZ_DATA_KEY_FILE=/data/paycheckzero.key
VOLUME /data
EXPOSE 8080
CMD ["paycheckzero"]
