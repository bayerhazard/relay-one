# Build stage: Rust server (static musl for small image)
FROM rust:1.91-alpine AS server-build
RUN apk add --no-cache musl-dev openssl-dev pkgconfig
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY server/Cargo.toml ./server/Cargo.toml
COPY server/src ./server/src
RUN cd server && cargo build --release --locked

# Build stage: web frontend
FROM node:22-alpine AS web-build
WORKDIR /app
COPY web/package.json web/package-lock.json ./
# --ignore-scripts: the "prepare" (svelte-kit sync) needs the source, which is
# copied in the next step. Running it during npm ci (source absent) would fail.
RUN npm ci --silent --ignore-scripts
COPY web/ ./
ENV NODE_ENV=production
RUN npm run prepare && npm run build

# Runtime
FROM alpine:3.21
RUN apk add --no-cache ca-certificates tzdata
WORKDIR /opt/relay
COPY --from=server-build /app/target/release/relay-server /opt/relay/relay-server
COPY --from=web-build /app/build /opt/relay/web
RUN mkdir -p /data/Relay && chown -R 1000:1000 /data/Relay /opt/relay
USER 1000
ENV RELAY_BIND=0.0.0.0:3000 \
    RELAY_DATA_DIR=/data/Relay \
    RELAY_WEB_DIR=/opt/relay/web
EXPOSE 3000
CMD ["/opt/relay/relay-server"]
