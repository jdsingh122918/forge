.PHONY: all build release test test-unit test-integration test-ui check fmt clippy clean install run help
.PHONY: dev dev-release factory factory-release down reset

# Default target
all: check build test

# ── Build ─────────────────────────────────────────────────────────────

build:
	cargo build

release:
	cargo build --release

build-ui:
	cd ui && npm ci && npm run build

# ── Test ──────────────────────────────────────────────────────────────

test:
	cargo test

test-unit:
	cargo test --lib

test-integration:
	cargo test --test integration_tests

test-ui:
	cd ui && npx vitest run

test-all: test test-ui

# ── Code quality ──────────────────────────────────────────────────────

check: fmt-check clippy

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy -- -D warnings

# ── Factory (local, no Docker) ────────────────────────────────────────

# Dev mode: cargo watch (backend) + vite HMR (frontend)
dev:
	@./start.sh dev-local

# Release mode: optimized binary + vite preview
dev-release:
	@./start.sh release-local

# Run factory with debug build
factory: build
	FORGE_CMD=$$(pwd)/target/debug/forge cargo run -- factory --dev

# Run factory with release build
factory-release: release
	FORGE_CMD=$$(pwd)/target/release/forge ./target/release/forge factory --dev

# ── Factory (Docker) ──────────────────────────────────────────────────

docker-dev:
	@./start.sh dev

docker-prod:
	@./start.sh prod

docker-down:
	@./start.sh down

# ── Cleanup ───────────────────────────────────────────────────────────

# Stop factory processes on ports 3141 and 5173
down:
	@echo "Stopping factory processes..."
	@-lsof -ti:3141 | xargs kill -9 2>/dev/null || true
	@-lsof -ti:5173 | xargs kill -9 2>/dev/null || true
	@echo "Done."

# Reset factory database (stops servers first)
reset: down
	@echo "Resetting factory database..."
	@rm -f .forge/factory.db .forge/factory.db-journal .forge/factory.db-wal .forge/factory.db-shm
	@echo "Cleaning up worktrees..."
	@find .forge/repos -name ".worktrees" -type d -exec rm -rf {} + 2>/dev/null || true
	@echo "Removing forge/* branches..."
	@git branch 2>/dev/null | grep "forge/" | xargs git branch -D 2>/dev/null || true
	@echo "Factory reset complete."

clean:
	cargo clean

# ── Install ───────────────────────────────────────────────────────────

install:
	cargo install --path .

# Run (use: make run ARGS="init")
run:
	cargo run -- $(ARGS)

# ── Help ──────────────────────────────────────────────────────────────

help:
	@echo "Available targets:"
	@echo ""
	@echo "  Build:"
	@echo "    build            Build debug binary"
	@echo "    release          Build release binary"
	@echo "    build-ui         Build frontend (npm ci + build)"
	@echo ""
	@echo "  Test:"
	@echo "    test             Run Rust tests"
	@echo "    test-unit        Run unit tests only"
	@echo "    test-integration Run integration tests only"
	@echo "    test-ui          Run frontend tests (vitest)"
	@echo "    test-all         Run all tests (Rust + frontend)"
	@echo ""
	@echo "  Quality:"
	@echo "    check            Run fmt-check and clippy"
	@echo "    fmt              Format code"
	@echo "    clippy           Run clippy linter"
	@echo ""
	@echo "  Factory (local):"
	@echo "    dev              Dev mode: cargo watch + vite HMR"
	@echo "    dev-release      Release mode: optimized binary + vite"
	@echo "    factory          Run factory (debug build)"
	@echo "    factory-release  Run factory (release build)"
	@echo ""
	@echo "  Factory (Docker):"
	@echo "    docker-dev       Dev mode via Docker Compose"
	@echo "    docker-prod      Production mode via Docker Compose"
	@echo "    docker-down      Stop Docker containers"
	@echo ""
	@echo "  Cleanup:"
	@echo "    down             Stop factory processes (ports 3141/5173)"
	@echo "    reset            Stop + wipe DB + clean worktrees/branches"
	@echo "    clean            Remove build artifacts (cargo clean)"
	@echo ""
	@echo "  Other:"
	@echo "    install          Install binary to ~/.cargo/bin"
	@echo "    run ARGS=\"...\"   Run with arguments"
