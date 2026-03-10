# Forge Context Graph (qmd-backed)

This graph was derived by indexing the repository with `qmd` and tracing subsystem links from:

- `src/main.rs`, `src/cmd/*`
- `src/orchestrator/*`, `src/dag/*`, `src/review/*`, `src/hooks/*`, `src/swarm/*`
- `src/factory/*`
- `ui/src/app.tsx`, `ui/src/hooks/usemissioncontrol.ts`, `ui/src/api/client.ts`, `ui/src/contexts/websocketcontext.tsx`

## Mermaid Graph

```mermaid
flowchart LR
    subgraph CLI["CLI Layer"]
        MAIN["src/main.rs"]
        CMD["src/cmd/*"]
    end

    subgraph CORE["Core Orchestration"]
        PHASE["src/phase.rs"]
        CFG["src/forge_config.rs"]
        ORCH["src/orchestrator/runner.rs"]
        OSTATE["src/orchestrator/state.rs"]
        HOOKS["src/hooks/*"]
        SIGNALS["src/signals/*"]
        OREVIEW["src/orchestrator/review_integration.rs"]
        REVIEW["src/review/*"]
        DAGS["src/dag/scheduler.rs"]
        DAGE["src/dag/executor.rs"]
        DECOMP["src/decomposition/*"]
        SWARM["src/swarm/executor.rs"]
    end

    subgraph FACTORY["Factory Backend"]
        FSERVER["src/factory/server.rs"]
        FAPI["src/factory/api.rs"]
        FPIPE["src/factory/pipeline.rs"]
        FDB["src/factory/db.rs"]
        FWS["src/factory/ws.rs"]
    end

    subgraph WEBUI["Factory React UI"]
        UAPP["ui/src/app.tsx"]
        UHOOK["ui/src/hooks/usemissioncontrol.ts"]
        UAPI["ui/src/api/client.ts"]
        UWS["ui/src/contexts/websocketcontext.tsx"]
    end

    MAIN --> CMD
    CMD --> ORCH
    CMD --> DAGE
    CMD --> FSERVER

    ORCH --> PHASE
    ORCH --> CFG
    ORCH --> OSTATE
    ORCH --> HOOKS
    ORCH --> SIGNALS
    ORCH --> OREVIEW
    OREVIEW --> REVIEW

    DAGE --> DAGS
    DAGS --> PHASE
    DAGE --> OREVIEW
    DAGE --> ORCH
    DAGE --> DECOMP
    HOOKS --> SWARM

    FSERVER --> FAPI
    FSERVER --> FWS
    FSERVER --> FPIPE
    FAPI --> FDB
    FAPI --> FPIPE
    FAPI --> FWS
    FPIPE --> FDB
    FPIPE --> FWS

    UAPP --> UHOOK
    UHOOK --> UAPI
    UHOOK --> UWS
    UAPI --> FAPI
    UWS --> FWS
```

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
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/orchestrator/mod.rs
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/factory/mod.rs
```
