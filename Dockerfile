# syntax=docker/dockerfile:1

FROM oven/bun:1-alpine AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package*.json ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

FROM rustlang/rust:nightly-alpine AS backend-builder
WORKDIR /app
RUN apk add --no-cache ca-certificates musl-dev pkgconfig openssl-dev openssl-libs-static

COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
COPY src ./src
RUN touch src/main.rs && cargo build --release \
    && mkdir -p /data/torrents

FROM scratch AS runtime
COPY --from=backend-builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=backend-builder /app/target/release/lantern /lantern
COPY --from=backend-builder /data /data
COPY --from=frontend-builder /app/frontend/dist /static

ENV LANTERN_BIND=0.0.0.0:3000
ENV LANTERN_STATIC_DIR=/static
ENV LANTERN_DATABASE_URL=sqlite:///data/lantern.db

EXPOSE 3000
ENTRYPOINT ["/lantern"]
CMD ["serve"]
