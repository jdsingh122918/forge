# Stage 1: Build the React UI
FROM node:22-alpine AS ui-builder
WORKDIR /app/ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci
COPY ui/ ./
RUN npm run build

# Stage 2: Build the Rust binary
FROM rust:1.93-slim-bookworm AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src
COPY --from=ui-builder /app/ui/dist ./ui/dist
COPY src ./src
RUN touch src/main.rs && cargo build --release

# Stage 3: Production runtime
FROM debian:bookworm-slim AS runtime
WORKDIR /app
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    git \
    curl \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*
RUN npm install -g @anthropic-ai/claude-code
COPY --from=builder /app/target/release/forge /usr/local/bin/forge
ENV CLAUDE_CMD=claude
ENV FORGE_CMD=forge
RUN useradd -m -u 1000 forge
USER forge
EXPOSE 3141
ENTRYPOINT ["forge"]
CMD ["factory"]

# Stage 4: Development environment
FROM rust:1.93-slim-bookworm AS dev
WORKDIR /app
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    git \
    curl \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-watch
RUN npm install -g @anthropic-ai/claude-code
ENV CLAUDE_CMD=claude
ENV FORGE_CMD=forge
RUN mkdir -p /app/.forge
EXPOSE 3141 5173
CMD ["cargo", "watch", "-i", ".forge/", "-x", "run -- factory --dev"]
