# Forge File-to-File Dependency Graph (Runtime Focus)

This is the higher-granularity companion to `docs/context-graph.md`, showing
the current execution-critical file links across CLI entrypoints, project
bootstrap, orchestration, DAG execution, reviews, factory backend, and the
React Mission Control UI.

Scope notes:

- Focused on control-flow-heavy files and current runtime boundaries.
- Uses current paths such as `src/factory/pipeline/mod.rs`,
  `src/factory/db/mod.rs`, `ui/src/App.tsx`, and
  `ui/src/contexts/WebSocketContext.tsx`.
- Omits tests and many leaf helpers to keep the graph readable.

## Mermaid Graph

```mermaid
flowchart LR
    subgraph CLI["CLI Entrypoints"]
        MAIN["src/main.rs"]
        CMDMOD["src/cmd/mod.rs"]
        CMDPROJECT["src/cmd/project.rs"]
        CMDRUN["src/cmd/run.rs"]
        CMDSWARM["src/cmd/swarm.rs"]
        CMDFACTORY["src/cmd/factory.rs"]
        CMDCOMPACT["src/cmd/compact.rs"]
        CMDPATTERN["src/cmd/patterns.rs"]
        CMDUPDATE["src/cmd/update.rs"]
        CMDAUTO["src/cmd/autoresearch/mod.rs"]
    end

    subgraph BOOT["Bootstrap, Config, and Planning"]
        INIT["src/init/mod.rs"]
        INTERVIEW["src/interview/mod.rs"]
        GENERATE["src/generate/mod.rs"]
        IMPLEMENT["src/implement/mod.rs"]
        CONFIG["src/config.rs"]
        FORGECFG["src/forge_config.rs"]
        PHASE["src/phase.rs"]
        PATTERNS["src/patterns/mod.rs"]
        UPDATECHK["src/update_check.rs"]
        AUTOLOOP["src/cmd/autoresearch/loop_runner.rs"]
        AUTOBENCH["src/autoresearch/benchmarks.rs"]
    end

    subgraph ORCH["Sequential Runtime"]
        ORRUN["src/orchestrator/runner.rs"]
        ORSTATE["src/orchestrator/state.rs"]
        OREVINT["src/orchestrator/review_integration.rs"]
        HOOKMAN["src/hooks/manager.rs"]
        HOOKEXEC["src/hooks/executor.rs"]
        GATES["src/gates/mod.rs"]
        SIGNALSP["src/signals/parser.rs"]
        COMPACTM["src/compaction/manager.rs"]
        AUDIT["src/audit/mod.rs"]
        GITTRACK["src/tracker/git.rs"]
        UIPROG["src/ui/progress.rs"]
        UIDAG["src/ui/dag_progress.rs"]
        COUNCILENG["src/council/engine.rs"]
    end

    subgraph DAG["Parallel Runtime and Reviews"]
        DAGEXE["src/dag/executor.rs"]
        DAGSCH["src/dag/scheduler.rs"]
        DECOMPEXE["src/decomposition/executor.rs"]
        REVDISP["src/review/dispatcher.rs"]
        REVSPEC["src/review/specialists.rs"]
        REVARB["src/review/arbiter.rs"]
        SWEXEC["src/swarm/executor.rs"]
    end

    subgraph FACTORY["Factory Backend"]
        FSERVER["src/factory/server.rs"]
        FAPI["src/factory/api.rs"]
        FPIPEMOD["src/factory/pipeline/mod.rs"]
        FPIPEEXEC["src/factory/pipeline/execution.rs"]
        FPIPEGIT["src/factory/pipeline/git.rs"]
        FPIPEPARSE["src/factory/pipeline/parsing.rs"]
        FPLANNER["src/factory/planner.rs"]
        FAGENT["src/factory/agent_executor.rs"]
        FDBMOD["src/factory/db/mod.rs"]
        FGH["src/factory/github.rs"]
        FWS["src/factory/ws.rs"]
        FMODEL["src/factory/models.rs"]
        FSANDBOX["src/factory/sandbox.rs"]
        FMETR["src/metrics/mod.rs"]
    end

    subgraph WEBUI["React UI"]
        UIMAIN["ui/src/main.tsx"]
        UIAPP["ui/src/App.tsx"]
        UIMISSION["ui/src/hooks/useMissionControl.ts"]
        UIAPI["ui/src/api/client.ts"]
        UIWS["ui/src/contexts/WebSocketContext.tsx"]
        UITYPES["ui/src/types/index.ts"]
    end

    MAIN --> CMDMOD
    MAIN --> UPDATECHK

    CMDMOD --> CMDPROJECT
    CMDMOD --> CMDRUN
    CMDMOD --> CMDSWARM
    CMDMOD --> CMDFACTORY
    CMDMOD --> CMDCOMPACT
    CMDMOD --> CMDPATTERN
    CMDMOD --> CMDUPDATE
    CMDMOD --> CMDAUTO

    CMDPROJECT --> INIT
    CMDPROJECT --> INTERVIEW
    CMDPROJECT --> GENERATE
    CMDPROJECT --> IMPLEMENT

    INTERVIEW --> INIT
    INTERVIEW --> FORGECFG
    GENERATE --> INIT
    GENERATE --> FORGECFG
    GENERATE --> PHASE
    IMPLEMENT --> INIT
    IMPLEMENT --> GENERATE
    IMPLEMENT --> PHASE

    CMDPATTERN --> PATTERNS
    CMDUPDATE --> UPDATECHK
    CMDAUTO --> AUTOLOOP
    CMDAUTO --> AUTOBENCH

    CMDRUN --> CONFIG
    CMDRUN --> FORGECFG
    CMDRUN --> PHASE
    CMDRUN --> ORRUN
    CMDRUN --> ORSTATE
    CMDRUN --> OREVINT
    CMDRUN --> HOOKMAN
    CMDRUN --> GATES
    CMDRUN --> COMPACTM
    CMDRUN --> AUDIT
    CMDRUN --> GITTRACK
    CMDRUN --> UIPROG

    CMDCOMPACT --> FORGECFG
    CMDCOMPACT --> ORSTATE
    CMDCOMPACT --> COMPACTM

    CONFIG --> FORGECFG
    ORRUN --> PHASE
    ORRUN --> SIGNALSP
    ORRUN --> AUDIT
    ORRUN --> COUNCILENG
    ORRUN --> GITTRACK
    ORRUN --> UIPROG
    OREVINT --> PHASE
    OREVINT --> REVDISP
    OREVINT --> REVSPEC
    HOOKMAN --> HOOKEXEC
    HOOKEXEC --> SWEXEC
    GATES --> PHASE

    CMDSWARM --> PHASE
    CMDSWARM --> DAGSCH
    CMDSWARM --> DAGEXE
    CMDSWARM --> OREVINT
    CMDSWARM --> UIDAG

    DAGEXE --> DAGSCH
    DAGEXE --> ORRUN
    DAGEXE --> OREVINT
    DAGEXE --> DECOMPEXE
    DAGEXE --> GITTRACK
    DAGEXE --> FORGECFG
    DAGSCH --> PHASE
    REVDISP --> REVSPEC
    REVDISP --> REVARB

    CMDFACTORY --> FSERVER

    FSERVER --> FAPI
    FSERVER --> FPIPEMOD
    FSERVER --> FDBMOD
    FSERVER --> FWS
    FSERVER --> FSANDBOX
    FSERVER --> FMETR

    FAPI --> FDBMOD
    FAPI --> FPIPEMOD
    FAPI --> FGH
    FAPI --> FWS
    FAPI --> FMODEL
    FAPI --> FMETR

    FPIPEMOD --> FPIPEEXEC
    FPIPEMOD --> FPIPEGIT
    FPIPEMOD --> FPIPEPARSE
    FPIPEMOD --> FPLANNER
    FPIPEMOD --> FAGENT
    FPIPEMOD --> FDBMOD
    FPIPEMOD --> FWS
    FPIPEMOD --> FMODEL
    FPIPEMOD --> FSANDBOX
    FPIPEMOD --> FMETR
    FAGENT --> FPIPEPARSE
    FMETR --> FDBMOD

    UIMAIN --> UIAPP
    UIAPP --> UIMISSION
    UIAPP --> UIAPI
    UIAPP --> UIWS
    UIMISSION --> UIAPI
    UIMISSION --> UIWS
    UIMISSION --> UITYPES
    UIAPI --> UITYPES
    UIWS --> UITYPES

    UIAPI -->|"/api/*"| FAPI
    UIWS -->|"/ws"| FWS
```

## qmd Walkthrough

```bash
export XDG_CONFIG_HOME=/tmp/qmdcfg
export XDG_CACHE_HOME=/tmp/qmdcache
export TMPDIR=/tmp

bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/cmd/run.rs:1 -l 220
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/dag/executor.rs:1 -l 260
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/factory/pipeline/mod.rs:1 -l 320
bunx @tobilu/qmd --index forge-context get qmd://forge-rust/src/factory/server.rs:1 -l 240
bunx @tobilu/qmd --index forge-context get qmd://forge-ui/ui/src/App.tsx:1 -l 220
bunx @tobilu/qmd --index forge-context get qmd://forge-ui/ui/src/hooks/useMissionControl.ts:1 -l 260
```
