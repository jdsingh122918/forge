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

# Warn if no authentication is configured
has_api_key=false
has_oauth=false
if [ -n "${ANTHROPIC_API_KEY:-}" ] && [ "$ANTHROPIC_API_KEY" != "your-api-key-here" ]; then
    has_api_key=true
fi
if [ -n "${CLAUDE_CODE_OAUTH_TOKEN:-}" ] && [ "$CLAUDE_CODE_OAUTH_TOKEN" != "your-oauth-token-here" ]; then
    has_oauth=true
fi
if [ "$has_api_key" = false ] && [ "$has_oauth" = false ]; then
    echo "WARNING: No authentication configured. Pipeline execution will not work."
    echo "         Edit .env and set ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN."
    echo "         (Run 'claude setup-token' to generate an OAuth token)"
    echo ""
fi

# Inform about optional GitHub integration
if [ -z "${GITHUB_TOKEN:-}" ] && [ -z "${GITHUB_CLIENT_ID:-}" ]; then
    echo "NOTE: GitHub integration not configured (optional)."
    echo "      Set GITHUB_TOKEN and/or GITHUB_CLIENT_ID in .env for GitHub features."
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
