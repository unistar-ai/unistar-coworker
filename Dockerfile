# syntax=docker/dockerfile:1

# Stage 1 — React Web UI (Vite)
FROM node:22-bookworm-slim AS web-ui
WORKDIR /build/web-ui
COPY web-ui/package.json web-ui/package-lock.json ./
RUN npm ci
COPY web-ui/ ./
RUN npm run build:fast

# Stage 2 — Rust release binary (embed-web-ui, no Chromium)
FROM rust:bookworm AS builder
WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends mold pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
COPY vendor ./vendor
COPY prompts ./prompts
COPY coworker.example.yaml ./
COPY --from=web-ui /build/web-ui/dist ./web-ui/dist

RUN cargo build --release --no-default-features --features embed-web-ui -p unistar-coworker

# Stage 3 — minimal runtime
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/unistar-coworker /usr/local/bin/unistar-coworker

WORKDIR /app
COPY skills ./skills
COPY packaging/workdir-template ./template
COPY docs ./docs
COPY README.md README_CN.md QUICKSTART.md QUICKSTART_CN.md coworker.example.yaml ./

EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/unistar-coworker"]
CMD ["serve"]
