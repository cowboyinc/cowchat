# Build stage
FROM rust:1.85-bookworm AS builder

WORKDIR /app
COPY . .

RUN cargo build --release -p cowchat-server

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/cowchat-server /usr/local/bin/cowchat-server

# Create data directory
RUN mkdir -p /data

EXPOSE 8080

CMD ["cowchat-server", "serve", \
     "--http", "0.0.0.0:8080", \
     "--no-tcp", \
     "--db", "/data/cowchat.db", \
     "--key-file", "/data/auth.key", \
     "--socket", "/tmp/cowchat.sock"]
