FROM lukemathwalker/cargo-chef:latest-rust-alpine AS chef
WORKDIR /app
# Install build dependencies (needed for aws-lc-rs/openssl/etc)
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    openssl-dev \
    cmake \
    make \
    g++ \
    perl \
    git

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release

# Runtime Stage
FROM alpine:latest
# Install runtime dependencies
RUN apk add --no-cache \
    libgcc \
    libstdc++ \
    ca-certificates \
    tzdata

WORKDIR /app
COPY --from=builder /app/target/release/ad-nope .
COPY config.toml .

EXPOSE 5300/udp
EXPOSE 5300/tcp

ENTRYPOINT ["./ad-nope"]
