FROM rust:1.96-slim-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin app \
    && mkdir -p /data \
    && chown -R app:app /data

ENV DATA_DIR=/data
ENV BIND_ADDR=0.0.0.0:3000
ENV RUST_LOG=jmcomic_bot_service=info,tower_http=info

COPY --from=builder /app/target/release/jmcomic-bot-service /usr/local/bin/jmcomic-bot-service

USER app
VOLUME ["/data"]
EXPOSE 3000

CMD ["jmcomic-bot-service"]
