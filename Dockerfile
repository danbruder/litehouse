# Frontend build stage
FROM node:20-alpine AS frontend-builder

WORKDIR /build/assets

# Install Elm
RUN npm install -g elm

# Copy Elm source files
COPY assets/elm.json ./
COPY assets/src ./src
COPY assets/public ./public

# Build Elm app
RUN mkdir -p dist && \
    elm make src/Main.elm --optimize --output=dist/app.js && \
    cp public/index.html dist/index.html

# Backend build stage
FROM rust:1.92-alpine AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconfig

WORKDIR /build

# Copy project files
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY .sqlx ./.sqlx

# Copy built frontend assets
COPY --from=frontend-builder /build/assets/dist ./assets/dist

# Build static binary
ENV SQLX_OFFLINE=true
ENV RUSTFLAGS="-C target-feature=+crt-static"
RUN cargo build --release --target x86_64-unknown-linux-musl

# Runtime stage
FROM alpine:latest

# Install runtime dependencies (git for cloning repos, podman CLI for builds)
RUN apk add --no-cache \
    ca-certificates \
    git

# Create litehouse user and directories
RUN addgroup -g 1000 litehouse && \
    adduser -D -u 1000 -G litehouse litehouse

# Create necessary directories
RUN mkdir -p /opt/litehouse/config /opt/litehouse/data && \
    chown -R litehouse:litehouse /opt/litehouse

# Copy binary from builder
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/lh /usr/local/bin/lh

# Set working directory
WORKDIR /opt/litehouse

# Run as litehouse user
USER litehouse

# Expose API port (3030 only - Caddy handles 80/443)
EXPOSE 3030

# Default command
CMD ["lh", "serve"]
