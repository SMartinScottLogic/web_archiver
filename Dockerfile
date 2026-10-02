FROM rust:1-bookworm AS build

WORKDIR /build

RUN apt-get update \
    && apt-get install --no-install-recommends -y pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .
RUN cargo build --locked --release --package web_archiver

FROM debian:bookworm-slim AS run

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 archiver \
    && useradd --uid 10001 --gid archiver --no-create-home archiver \
    && mkdir -p /data \
    && chown archiver:archiver /data

WORKDIR /app
COPY --from=build /build/target/release/web_archiver /usr/local/bin/web_archiver

ENV RUST_LOG=info
VOLUME ["/data"]
USER archiver

ENTRYPOINT ["/usr/local/bin/web_archiver"]
CMD ["--archive-dir", "/data/archive", "--db", "/data/crawler.db"]
