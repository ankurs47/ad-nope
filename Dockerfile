# Build Stage
FROM rust:alpine AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    openssl-dev \
    cmake \
    make \
    g++ \
    perl \
    git

WORKDIR /app

# Copy the source code
COPY . .

# Build the application in release mode
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

# Copy the binary from the builder stage
COPY --from=builder /app/target/release/ad-nope .
# Copy the default configuration
COPY config.toml .

# Expose DNS ports (Default in config.toml is 5300)
EXPOSE 5300/udp
EXPOSE 5300/tcp

# Run the application
ENTRYPOINT ["./ad-nope"]
