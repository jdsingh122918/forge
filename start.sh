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
    echo "  (none)         Start Forge in production mode (Docker)"
    echo "  prod           Same as above"
    echo "  dev            Start in dev mode (Docker: cargo watch + vite HMR)"
    echo "  dev-local      Start in dev mode (local: cargo watch + vite HMR)"
    echo "  release-local  Start in release mode (local: optimized binary + vite)"
    echo "  build          Build production Docker image"
    echo "  down           Stop all (Docker containers + local processes)"
    echo "  reset          Stop all + wipe database + clean worktrees/branches"
    echo ""
}

# Stop local processes on factory ports
stop_local() {
    lsof -ti:3141 | xargs kill -9 2>/dev/null || true
    lsof -ti:5173 | xargs kill -9 2>/dev/null || true
}

# Ensure frontend dependencies are installed
ensure_ui_deps() {
    if [ ! -d ui/node_modules ]; then
        echo "Installing frontend dependencies..."
        (cd ui && npm ci)
    fi
}

case "${1:-prod}" in
    prod)
        echo "Starting Forge (production mode) on http://localhost:3141 ..."
        docker compose --profile prod up --build
        ;;
    dev)
        echo "Starting Forge (dev mode — Docker)..."
        echo "  Backend (cargo watch): http://localhost:3141"
        echo "  Frontend (vite HMR):   http://localhost:5173"
        docker compose --profile dev up --build
        ;;
    dev-local)
        stop_local
        ensure_ui_deps
        export FORGE_CMD="${FORGE_CMD:-$SCRIPT_DIR/target/debug/forge}"
        echo "Starting Forge (dev mode — local)..."
        echo "  Backend (cargo watch): http://localhost:3141"
        echo "  Frontend (vite HMR):   http://localhost:5173"
        echo "  FORGE_CMD=$FORGE_CMD"
        echo ""

        # Start frontend in background
        (cd ui && npm run dev) &
        UI_PID=$!

        # Start backend with cargo watch (rebuilds on change)
        cargo watch \
            -i ".forge/" -i "ui/" -i ".claude/" -i ".entire/" -i ".git/" \
            -x "run -- factory --dev" &
        BACKEND_PID=$!

        # Wait for either to exit, then clean up
        trap "kill $UI_PID $BACKEND_PID 2>/dev/null; exit" INT TERM
        wait -n $UI_PID $BACKEND_PID 2>/dev/null
        kill $UI_PID $BACKEND_PID 2>/dev/null
        ;;
    release-local)
        stop_local
        ensure_ui_deps
        echo "Building release binary..."
        cargo build --release
        export FORGE_CMD="${FORGE_CMD:-$SCRIPT_DIR/target/release/forge}"
        echo ""
        echo "Starting Forge (release mode — local)..."
        echo "  Backend: http://localhost:3141"
        echo "  Frontend (vite HMR): http://localhost:5173"
        echo "  FORGE_CMD=$FORGE_CMD"
        echo ""

        # Start frontend in background
        (cd ui && npm run dev) &
        UI_PID=$!

        # Start backend with release binary
        ./target/release/forge factory --dev &
        BACKEND_PID=$!

        trap "kill $UI_PID $BACKEND_PID 2>/dev/null; exit" INT TERM
        wait -n $UI_PID $BACKEND_PID 2>/dev/null
        kill $UI_PID $BACKEND_PID 2>/dev/null
        ;;
    build)
        echo "Building production image..."
        docker compose --profile prod build
        ;;
    down)
        echo "Stopping all processes..."
        stop_local
        docker compose --profile prod --profile dev down 2>/dev/null || true
        echo "All processes stopped."
        ;;
    reset)
        echo "Stopping all processes..."
        stop_local
        docker compose --profile prod --profile dev down 2>/dev/null || true

        echo "Resetting factory database..."
        rm -f .forge/factory.db .forge/factory.db-journal .forge/factory.db-wal .forge/factory.db-shm

        echo "Cleaning up worktrees..."
        find .forge/repos -name ".worktrees" -type d -exec rm -rf {} + 2>/dev/null || true

        echo "Removing forge/* branches..."
        git branch 2>/dev/null | grep "forge/" | xargs git branch -D 2>/dev/null || true

        echo "Factory reset complete."
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
