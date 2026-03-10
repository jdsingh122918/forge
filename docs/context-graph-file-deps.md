# Forge File-to-File Dependency Graph (Runtime Focus)

This is a higher-granularity graph than `docs/context-graph.md`, showing key runtime file-to-file links (imports/usages) across CLI, orchestrator, DAG, review, swarm, factory backend, and React UI.

Scope notes:

- Focused on execution-critical files.
- Omits tests and many leaf utility files to keep the graph readable.

## Mermaid Graph

```mermaid
flowchart LR
    subgraph CLI["CLI Entrypoints"]
        MAIN["src/main.rs"]
        CMDMOD["src/cmd/mod.rs"]
        CMDRUN["src/cmd/run.rs"]
        CMDSWARM["src/cmd/swarm.rs"]
        CMDFACTORY["src/cmd/factory.rs"]
    end

    subgraph ORCH["Sequential Orchestration Path"]
        ORRUN["src/orchestrator/runner.rs"]
        ORSTATE["src/orchestrator/state.rs"]
        OREVINT["src/orchestrator/review_integration.rs"]
        PHASE["src/phase.rs"]
        FCFG["src/forge_config.rs"]
        HOOKMAN["src/hooks/manager.rs"]
        HOOKEXEC["src/hooks/executor.rs"]
        GATES["src/gates/mod.rs"]
        SIGNALSP["src/signals/parser.rs"]
        AUDITLOG["src/audit/logger.rs"]
        GITTRACK["src/tracker/git.rs"]
        SKILLS["src/skills/mod.rs"]
        UIPROG["src/ui/progress.rs"]
    end

    subgraph DAG["Parallel DAG Path"]
        DAGEXE["src/dag/executor.rs"]
        DAGSCH["src/dag/scheduler.rs"]
        DAGSTATE["src/dag/state.rs"]
        DECOMPEXE["src/decomposition/executor.rs"]
    end

    subgraph REVIEW["Review System"]
        REVDISP["src/review/dispatcher.rs"]
        REVSPEC["src/review/specialists.rs"]
        REVARB["src/review/arbiter.rs"]
    end

    subgraph SWARM["Swarm Hook Runtime"]
        SWEXEC["src/swarm/executor.rs"]
        SWCALL["src/swarm/callback.rs"]
        SWCTX["src/swarm/context.rs"]
        SWPROMPT["src/swarm/prompts.rs"]
    end

    subgraph FACTORY["Factory Backend"]
        FSERVER["src/factory/server.rs"]
        FAPI["src/factory/api.rs"]
        FPIPE["src/factory/pipeline.rs"]
        FDB["src/factory/db.rs"]
        FWS["src/factory/ws.rs"]
        FMODEL["src/factory/models.rs"]
        FAGENT["src/factory/agent_executor.rs"]
        FPLANNER["src/factory/planner.rs"]
        FSANDBOX["src/factory/sandbox.rs"]
        FMETR["src/metrics/mod.rs"]
    end

    subgraph WEBUI["React UI"]
        UIMAIN["ui/src/main.tsx"]
        UIAPP["ui/src/app.tsx"]
        UIMISSION["ui/src/hooks/usemissioncontrol.ts"]
        UIAPI["ui/src/api/client.ts"]
        UIWS["ui/src/contexts/websocketcontext.tsx"]
        UITYPES["ui/src/types/index.ts"]
    end

    MAIN --> CMDMOD
    CMDMOD --> CMDRUN
    CMDMOD --> CMDSWARM
    CMDMOD --> CMDFACTORY

    CMDRUN --> ORRUN
    CMDRUN --> ORSTATE
    CMDRUN --> PHASE
    CMDRUN --> FCFG
    CMDRUN --> HOOKMAN
    CMDRUN --> GATES
    CMDRUN --> AUDITLOG
    CMDRUN --> GITTRACK

    CMDSWARM --> DAGEXE
    CMDSWARM --> DAGSCH
    CMDSWARM --> PHASE
    CMDSWARM --> OREVINT

    CMDFACTORY --> FSERVER

    ORRUN --> PHASE
    ORRUN --> FCFG
    ORRUN --> SIGNALSP
    ORRUN --> SKILLS
    ORRUN --> UIPROG
    ORRUN --> AUDITLOG

    HOOKMAN --> HOOKEXEC
    HOOKEXEC --> SWEXEC
    GATES --> PHASE
    GATES --> FCFG

    OREVINT --> PHASE
    OREVINT --> REVDISP
    OREVINT --> REVSPEC

    DAGEXE --> DAGSCH
    DAGEXE --> DAGSTATE
    DAGEXE --> ORRUN
    DAGEXE --> OREVINT
    DAGEXE --> PHASE
    DAGEXE --> DECOMPEXE
    DAGEXE --> GITTRACK
    DAGEXE --> FCFG
    DAGSCH --> PHASE

    REVDISP --> REVSPEC
    REVDISP --> REVARB

    SWEXEC --> SWCALL
    SWEXEC --> SWCTX
    SWEXEC --> SWPROMPT

    FSERVER --> FAPI
    FSERVER --> FDB
    FSERVER --> FPIPE
    FSERVER --> FWS
    FSERVER --> FSANDBOX
    FSERVER --> FMETR

    FAPI --> FDB
    FAPI --> FPIPE
    FAPI --> FWS
    FAPI --> FMODEL
    FAPI --> FMETR

    FPIPE --> FAGENT
    FPIPE --> FDB
    FPIPE --> FMODEL
    FPIPE --> FPLANNER
    FPIPE --> FSANDBOX
    FPIPE --> FWS
    FPIPE --> FMETR

    FWS --> FAPI
    FWS --> FMODEL
    FDB --> FMODEL

    UIMAIN --> UIAPP
    UIAPP --> UIMISSION
    UIAPP --> UIWS
    UIAPP --> UIAPI
    UIMISSION --> UIAPI
    UIMISSION --> UIWS
    UIAPI --> UITYPES
    UIWS --> UITYPES

    UIAPI -->|"/api/*"| FAPI
    UIWS -->|"/ws"| FWS
```

## qmd Walkthrough (same index)

```bash
export XDG_CONFIG_HOME=/tmp/qmdcfg
export XDG_CACHE_HOME=/tmp/qmdcache
export TMPDIR=/tmp

bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/cmd/run.rs:1 -l 220
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/dag/executor.rs:1 -l 260
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/factory/pipeline.rs:1 -l 260
bunx @tobilu/qmd --index forge-context get qmd://forge-ui/ui/src/hooks/usemissioncontrol.ts:1 -l 220
```
