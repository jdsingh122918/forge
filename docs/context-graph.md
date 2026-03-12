# Forge Context Graph (qmd-backed)

This graph reflects the current repository layout. It was refreshed by indexing
the repo with `qmd` and tracing subsystem links from:

- `src/main.rs`, `src/lib.rs`, `src/cmd/*`
- `src/init/*`, `src/interview/*`, `src/generate/*`, `src/implement/*`
- `src/orchestrator/*`, `src/dag/*`, `src/decomposition/*`, `src/review/*`,
  `src/council/*`, `src/hooks/*`, `src/swarm/*`
- `src/audit/*`, `src/compaction/*`, `src/metrics/*`, `src/tracker/*`,
  `src/ui/*`, `src/update_check.rs`
- `src/factory/*`
- `ui/src/App.tsx`, `ui/src/hooks/useMissionControl.ts`,
  `ui/src/hooks/useAgentTeam.ts`, `ui/src/api/client.ts`,
  `ui/src/contexts/WebSocketContext.tsx`

## Mermaid Graph

```mermaid
flowchart LR
    subgraph CLI["CLI Layer"]
        MAIN["src/main.rs"]
        CMD["src/cmd/*"]
        TELE["src/telemetry.rs"]
        UPDCHK["src/update_check.rs"]
    end

    subgraph BOOT["Project Bootstrap & Planning"]
        INIT["src/init/*"]
        INTERVIEW["src/interview/*"]
        GENERATE["src/generate/*"]
        IMPLEMENT["src/implement/*"]
        CONFIG["src/config.rs + src/forge_config.rs"]
        PHASE["src/phase.rs"]
        PATTERNS["src/patterns/*"]
    end

    subgraph EXEC["Execution Engines"]
        ORCH["src/orchestrator/*"]
        DAG["src/dag/*"]
        DECOMP["src/decomposition/* + src/subphase/*"]
        REVIEW["src/review/*"]
        COUNCIL["src/council/*"]
        HOOKS["src/hooks/*"]
        SWARM["src/swarm/*"]
    end

    subgraph RUNTIME["Runtime State, Signals, and Metrics"]
        AUDIT["src/audit/*"]
        COMPACT["src/compaction/*"]
        SIGNALS["src/signals/* + src/stream/*"]
        TRACKER["src/tracker/*"]
        TUI["src/ui/*"]
        METRICS["src/metrics/*"]
    end

    subgraph FACTORY["Factory Backend"]
        FSERVER["src/factory/server.rs"]
        FAPI["src/factory/api.rs"]
        FPIPE["src/factory/pipeline/*"]
        FDB["src/factory/db/*"]
        FPLANNER["src/factory/planner.rs"]
        FAGENT["src/factory/agent_executor.rs"]
        FGH["src/factory/github.rs"]
        FWS["src/factory/ws.rs"]
        FSBOX["src/factory/sandbox.rs"]
        FMODEL["src/factory/models.rs"]
    end

    subgraph AUTO["Automated Research"]
        AUTOCMD["src/cmd/autoresearch/*"]
        AUTOLIB["src/autoresearch/*"]
    end

    subgraph WEBUI["Factory React UI"]
        UAPP["ui/src/App.tsx"]
        UHOOK["ui/src/hooks/useMissionControl.ts"]
        UHOOK2["ui/src/hooks/useAgentTeam.ts"]
        UAPI["ui/src/api/client.ts"]
        UWS["ui/src/contexts/WebSocketContext.tsx"]
        UTYPES["ui/src/types/index.ts"]
    end

    MAIN --> CMD
    MAIN --> TELE
    MAIN --> UPDCHK

    CMD --> INIT
    CMD --> INTERVIEW
    CMD --> GENERATE
    CMD --> IMPLEMENT
    CMD --> PATTERNS
    CMD --> ORCH
    CMD --> DAG
    CMD --> FSERVER
    CMD --> AUTOCMD

    INTERVIEW --> INIT
    INTERVIEW --> CONFIG
    GENERATE --> INIT
    GENERATE --> CONFIG
    GENERATE --> PHASE
    IMPLEMENT --> INIT
    IMPLEMENT --> GENERATE
    IMPLEMENT --> PHASE

    ORCH --> PHASE
    ORCH --> CONFIG
    ORCH --> HOOKS
    ORCH --> REVIEW
    ORCH --> COUNCIL
    ORCH --> AUDIT
    ORCH --> COMPACT
    ORCH --> SIGNALS
    ORCH --> TRACKER
    ORCH --> TUI

    DAG --> PHASE
    DAG --> ORCH
    DAG --> REVIEW
    DAG --> DECOMP
    DAG --> TRACKER
    HOOKS --> SWARM

    FSERVER --> FAPI
    FSERVER --> FPIPE
    FSERVER --> FDB
    FSERVER --> FWS
    FSERVER --> FSBOX
    FSERVER --> METRICS
    FAPI --> FDB
    FAPI --> FPIPE
    FAPI --> FGH
    FAPI --> FWS
    FAPI --> FMODEL
    FAPI --> METRICS
    FPIPE --> FPLANNER
    FPIPE --> FAGENT
    FPIPE --> FDB
    FPIPE --> FWS
    FPIPE --> FSBOX
    FPIPE --> FMODEL
    FPIPE --> METRICS
    METRICS --> FDB

    AUTOCMD --> AUTOLIB

    UAPP --> UHOOK
    UAPP --> UAPI
    UAPP --> UWS
    UHOOK --> UAPI
    UHOOK --> UWS
    UHOOK --> UTYPES
    UHOOK2 --> UAPI
    UHOOK2 --> UWS
    UHOOK2 --> UTYPES
    UAPI --> FAPI
    UWS --> FWS
```

## Notable Shifts

- Factory runtime is now split across `src/factory/pipeline/*` and
  `src/factory/db/*`, rather than single `pipeline.rs` / `db.rs` files.
- Sequential orchestration now has first-class council, compaction, audit, and
  signal-tracking paths.
- The UI entrypoints are the current case-sensitive files:
  `ui/src/App.tsx` and `ui/src/contexts/WebSocketContext.tsx`.

## Rebuild With qmd

```bash
# Isolated qmd config/cache in /tmp
export XDG_CONFIG_HOME=/tmp/qmdcfg
export XDG_CACHE_HOME=/tmp/qmdcache
export TMPDIR=/tmp

# Build/update a dedicated index for this repo
bunx @tobilu/qmd --index forge-context collection add . --name forge-rust --mask 'src/**/*.rs'
bunx @tobilu/qmd --index forge-context collection add . --name forge-docs --mask '**/*.md'
bunx @tobilu/qmd --index forge-context collection add . --name forge-ui --mask 'ui/src/**/*.{ts,tsx,js,jsx,css}'
bunx @tobilu/qmd --index forge-context update

# Explore files and relations
bunx @tobilu/qmd --index forge-context ls forge-rust
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/cmd/mod.rs:1 -l 220
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/orchestrator/runner.rs:1 -l 260
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/factory/pipeline/mod.rs:1 -l 260
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/council/engine.rs:1 -l 240
bunx @tobilu/qmd --index forge-context get qmd://forge-ui/ui/src/App.tsx:1 -l 220
```
