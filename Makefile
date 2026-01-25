.PHONY: all build release test test-unit test-integration check fmt clippy clean install run help

# Default target
all: check build test

# Build targets
build:
	cargo build

release:
	cargo build --release

# Test targets
test:
	cargo test

test-unit:
	cargo test --lib

test-integration:
	cargo test --test integration_tests

# Code quality
check: fmt-check clippy

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy -- -D warnings

# Clean
clean:
	cargo clean

# Install locally
install:
	cargo install --path .

# Run (use: make run ARGS="init")
run:
	cargo run -- $(ARGS)

# Help
help:
	@echo "Available targets:"
	@echo "  all              - Run check, build, and test"
	@echo "  build            - Build debug binary"
	@echo "  release          - Build release binary"
	@echo "  test             - Run all tests"
	@echo "  test-unit        - Run unit tests only"
	@echo "  test-integration - Run integration tests only"
	@echo "  check            - Run fmt-check and clippy"
	@echo "  fmt              - Format code"
	@echo "  fmt-check        - Check code formatting"
	@echo "  clippy           - Run clippy linter"
	@echo "  clean            - Remove build artifacts"
	@echo "  install          - Install binary locally"
	@echo "  run ARGS=\"...\"   - Run with arguments (e.g., make run ARGS=\"init\")"
	@echo "  help             - Show this help message"
