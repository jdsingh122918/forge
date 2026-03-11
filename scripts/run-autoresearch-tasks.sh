#!/usr/bin/env bash
set -euo pipefail

# Autoresearch task runner — executes all tasks in wave order
# Council is enabled via .forge/forge.toml

FORGE="./target/release/forge"
TASKS_DIR="docs/superpowers/specs/autoresearch-tasks"
LOG_DIR="/tmp/forge-autoresearch-logs"

mkdir -p "$LOG_DIR"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

run_task() {
    local task_file="$1"
    local task_name
    task_name=$(basename "$task_file" .md)
    local log_file="$LOG_DIR/${task_name}.log"

    echo -e "${CYAN}━━━ Starting: ${task_name} ━━━${NC}"

    # Step 1: Generate phases
    echo -e "  ${YELLOW}[1/3]${NC} Generating phases..."
    if ! $FORGE implement "${TASKS_DIR}/${task_file}" --autonomous --dry-run >> "$log_file" 2>&1; then
        echo -e "  ${RED}FAILED${NC} generating phases. See: $log_file"
        return 1
    fi

    # Step 2: Reset state
    echo -e "  ${YELLOW}[2/3]${NC} Resetting state..."
    $FORGE reset --force >> "$log_file" 2>&1

    # Step 3: Execute
    echo -e "  ${YELLOW}[3/3]${NC} Running phases..."
    if $FORGE run --autonomous --yes >> "$log_file" 2>&1; then
        echo -e "  ${GREEN}✓ DONE${NC}: ${task_name}"
    else
        echo -e "  ${RED}✗ FAILED${NC}: ${task_name}. See: $log_file"
        return 1
    fi

    echo ""
}

run_wave() {
    local wave_num="$1"
    shift
    local tasks=("$@")

    echo -e "${CYAN}══════════════════════════════════════${NC}"
    echo -e "${CYAN}  Wave ${wave_num} (${#tasks[@]} tasks)${NC}"
    echo -e "${CYAN}══════════════════════════════════════${NC}"
    echo ""

    for task in "${tasks[@]}"; do
        run_task "${task}" || {
            echo -e "${RED}Wave ${wave_num} halted due to failure in ${task}${NC}"
            exit 1
        }
    done

    echo -e "${GREEN}Wave ${wave_num} complete.${NC}"
    echo ""
}

# Ensure binary is built
echo "Building forge..."
cargo build --release 2>&1 | tail -1
echo ""

# Wave 1 (independent): T01, T04, T07, T09, T10
run_wave 1 \
    "T01-prompt-config-and-loader.md" \
    "T04-benchmark-types-and-loader.md" \
    "T07-judge-and-codex-cli.md" \
    "T09-cli-registration.md" \
    "T10-budget-tracker.md"

# Wave 2 (depends on Wave 1): T02, T05a-d, T12, T14
run_wave 2 \
    "T02-extract-prompt-files.md" \
    "T05a-security-benchmarks.md" \
    "T05b-architecture-benchmarks.md" \
    "T05c-performance-benchmarks.md" \
    "T05d-simplicity-benchmarks.md" \
    "T12-results-tsv.md" \
    "T14-git-integration.md"

# Wave 3: T03, T06, T06b, T08
run_wave 3 \
    "T03-wire-into-dispatcher.md" \
    "T06-finding-matcher-and-scorer.md" \
    "T06b-benchmark-runner.md" \
    "T08-wire-judge-into-scorer.md"

# Wave 4: T11
run_wave 4 \
    "T11-single-experiment.md"

# Wave 5: T13
run_wave 5 \
    "T13-full-loop-orchestration.md"

echo -e "${GREEN}══════════════════════════════════════${NC}"
echo -e "${GREEN}  All tasks complete!${NC}"
echo -e "${GREEN}══════════════════════════════════════${NC}"
echo "Logs: $LOG_DIR/"
