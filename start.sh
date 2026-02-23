#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Create .env from template if it doesn't exist
if [ ! -f .env ]; then
    if [ -f .env.example ]; then
        cp .env.example .env
        echo "Created .env from .env.example — please edit it and set ANTHROPIC_API_KEY"
    fi
fi

# Source .env if it exists
if [ -f .env ]; then
    set -a
    source .env
    set +a
fi

# Warn if API key is missing
if [ -z "${ANTHROPIC_API_KEY:-}" ] || [ "$ANTHROPIC_API_KEY" = "your-api-key-here" ]; then
    echo "WARNING: ANTHROPIC_API_KEY is not set. Pipeline execution will not work."
    echo "         Edit .env and set your Anthropic API key."
    echo ""
fi

usage() {
    echo "Usage: ./start.sh [command]"
    echo ""
    echo "Commands:"
    echo "  (none)    Start Forge in production mode (build + run)"
    echo "  prod      Same as above"
    echo "  dev       Start in development mode (hot-reload)"
    echo "  build     Build production image without starting"
    echo "  down      Stop and remove all containers"
    echo ""
}

case "${1:-prod}" in
    prod)
        echo "Starting Forge (production mode) on http://localhost:3141 ..."
        docker compose --profile prod up --build
        ;;
    dev)
        echo "Starting Forge (dev mode)..."
        echo "  Backend (cargo watch): http://localhost:3141"
        echo "  Frontend (vite HMR):   http://localhost:5173"
        docker compose --profile dev up --build
        ;;
    build)
        echo "Building production image..."
        docker compose --profile prod build
        ;;
    down)
        docker compose --profile prod --profile dev down
        echo "All containers stopped."
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        echo "Unknown command: $1"
        usage
        exit 1
        ;;
esac
