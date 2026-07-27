# Stage 1: Build the Rust application
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

# Copy source and build real binary
COPY src ./src
RUN touch src/main.rs && cargo build --release

# Stage 2: Minimal runtime image
FROM alpine:3.19
RUN apk add --no-cache ca-certificates

WORKDIR /app
COPY --from=builder /app/target/release/unifi_scoped_proxy /app/

# Expose port
EXPOSE 8080

# Default environment variables
ENV UNIFI_BASE_URL=https://192.168.1.1
ENV LISTEN_ADDR=0.0.0.0:8080
ENV ACCEPT_INVALID_CERTS=true

CMD ["/app/unifi_scoped_proxy"]
