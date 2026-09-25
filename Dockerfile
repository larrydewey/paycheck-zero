# Optional container image (spec §7.6). The preferred deployment is the single
# `paycheckzero` binary plus a SQLite file; this image wraps exactly that.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p paycheckzero-web --bin paycheckzero

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /data paycheckzero && mkdir -p /data && chown paycheckzero /data
COPY --from=build /src/target/release/paycheckzero /usr/local/bin/paycheckzero
USER paycheckzero
ENV PZ_BIND=0.0.0.0:8080 \
    PZ_DATABASE_URL=sqlite:///data/paycheckzero.db?mode=rwc
VOLUME /data
EXPOSE 8080
CMD ["paycheckzero"]
