# Forge Swarm Orchestration Design

**Date:** 2026-01-26
**Status:** Draft
**Author:** Claude + User

---

## Executive Summary

This document describes the design for integrating Claude Code's swarm orchestration capabilities into Forge. The chosen approach (Approach B: Claude-as-Orchestrator) positions Forge as a **blueprint generator and monitor** while Claude Code handles the actual multi-agent coordination.

This enables:
- **Parallel phase execution** based on dependency graph analysis
- **Quality gates** via parallel review specialists (security, performance, architecture)
- **Dynamic decomposition** when phases prove more complex than expected
- **LLM-driven decisions** for autonomous resolution of review failures
- **Robust recovery** from crashes and failures via checkpointing

---

## Table of Contents

1. [Goals & Non-Goals](#goals--non-goals)
2. [Architecture Overview](#architecture-overview)
3. [Blueprint Format](#blueprint-format)
4. [Orchestration Prompt](#orchestration-prompt)
5. [Monitoring & State Sync](#monitoring--state-sync)
6. [Review Specialist Integration](#review-specialist-integration)
7. [LLM Arbiter](#llm-arbiter)
8. [Dynamic Decomposition](#dynamic-decomposition)
9. [CLI Interface](#cli-interface)
10. [Error Handling & Recovery](#error-handling--recovery)
11. [Implementation Plan](#implementation-plan)
12. [Alternatives Considered](#alternatives-considered)

---

## Goals & Non-Goals

### Goals

1. **Faster execution** - Run independent phases in parallel to reduce wall-clock time
2. **Better quality** - Parallel review agents gate phase completion
3. **Smarter decomposition** - Automatically split complex phases
4. **Autonomous operation** - LLM arbiter can resolve review failures without human intervention
5. **Resilience** - Recover gracefully from crashes and failures
6. **Backwards compatibility** - Existing `forge run` continues to work

### Non-Goals

- Replacing Claude Code's swarm implementation with a native one
- Supporting non-Claude LLM backends
- Real-time collaboration between human and swarm
- Distributed execution across multiple machines

---

## Architecture Overview

Forge becomes a **swarm blueprint generator and monitor**. Instead of running Claude directly for each phase, Forge:

1. **Analyzes** the phase DAG to identify parallelization opportunities
2. **Generates** a structured swarm blueprint (team configuration, tasks, dependencies)
3. **Launches** Claude Code with an orchestration prompt that uses TeammateTool
4. **Monitors** the swarm execution via team files and inboxes
5. **Aggregates** results back into Forge's state system

```
┌─────────────────────────────────────────────────────────────────┐
│                         FORGE CLI                                │
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │   Analyzer   │───▶│  Blueprint   │───▶│   Launcher   │       │
│  │              │    │  Generator   │    │              │       │
│  │ - Parse DAG  │    │ - Team spec  │    │ - Spawn CC   │       │
│  │ - Find ||    │    │ - Task list  │    │ - Pass stdin │       │
│  │ - Group work │    │ - Reviewers  │    │ - Monitor    │       │
│  └──────────────┘    └──────────────┘    └──────────────┘       │
│                                                 │                │
│                                                 ▼                │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                   CLAUDE CODE SWARM                       │   │
│  │                                                           │   │
│  │   Leader (orchestrator)                                   │   │
│  │      ├── Phase Workers (parallel execution)               │   │
│  │      ├── Review Specialists (quality gates)               │   │
│  │      └── Decomposition Agents (dynamic splitting)         │   │
│  │                                                           │   │
│  │   Communicates via: TeammateTool, TaskCreate/Update       │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                 │                │
│                                                 ▼                │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │    Inbox     │───▶│    State     │───▶│    Audit     │       │
│  │   Watcher    │    │   Updater    │    │   Logger     │       │
│  └──────────────┘    └──────────────┘    └──────────────┘       │
└─────────────────────────────────────────────────────────────────┘
```

**Key insight:** Forge doesn't need to understand *how* Claude Code coordinates agents—it just needs to express *what* should happen and monitor *that* it happened.

---

## Blueprint Format

The blueprint is a structured JSON document that Forge generates and passes to Claude Code.

```json
{
  "version": "1.0",
  "project": {
    "name": "my-project",
    "spec_hash": "abc123",
    "working_dir": "/path/to/project"
  },
  "team": {
    "name": "forge-run-20260126-143022",
    "description": "Forge orchestrated run for my-project"
  },
  "execution": {
    "mode": "parallel",
    "max_concurrent": 4,
    "backend": "auto"
  },
  "phases": [
    {
      "id": "01",
      "name": "Project scaffolding",
      "promise": "SCAFFOLD COMPLETE",
      "budget": 12,
      "depends_on": [],
      "skills": ["rust-conventions"],
      "permission_mode": "standard",
      "agent_type": "general-purpose"
    },
    {
      "id": "02",
      "name": "Database schema",
      "promise": "DB COMPLETE",
      "budget": 15,
      "depends_on": ["01"],
      "skills": ["sql-best-practices"],
      "permission_mode": "strict",
      "agent_type": "general-purpose"
    },
    {
      "id": "03",
      "name": "Config module",
      "promise": "CONFIG COMPLETE",
      "budget": 10,
      "depends_on": ["01"],
      "skills": [],
      "permission_mode": "standard",
      "agent_type": "general-purpose"
    }
  ],
  "reviews": {
    "enabled": true,
    "trigger": "phase_complete",
    "specialists": [
      {"type": "security-sentinel", "gate": true},
      {"type": "performance-oracle", "gate": false}
    ],
    "resolution": {
      "mode": "arbiter",
      "arbiter_config": {
        "model": "sonnet",
        "max_fix_attempts": 2,
        "escalate_on": ["critical_security", "data_loss_risk"],
        "auto_proceed_on": ["style", "minor_performance"]
      }
    }
  },
  "decomposition": {
    "enabled": true,
    "threshold": "budget > 15 OR complexity_signals"
  },
  "callbacks": {
    "on_phase_complete": "forge callback phase-complete",
    "on_review_finding": "forge callback review-finding",
    "on_blocker": "forge callback blocker"
  }
}
```

### Key Fields

- **phases**: Directly maps from `phases.json` with DAG encoded in `depends_on`
- **reviews**: Configures parallel review specialists and whether they gate progress
- **decomposition**: Enables dynamic phase splitting when complexity is detected
- **callbacks**: Shell commands Forge uses to receive real-time updates

---

## Orchestration Prompt

When Forge launches Claude Code, it passes a carefully crafted prompt that instructs the leader agent how to execute the blueprint.

```markdown
# Forge Swarm Orchestrator

You are the leader agent for a Forge orchestration run. Your job is to execute
the phases in the attached blueprint using Claude Code's swarm capabilities.

## Blueprint
<blueprint>
{blueprint_json}
</blueprint>

## Your Responsibilities

1. **Create the team**: `Teammate({ operation: "spawnTeam", team_name: "{team_name}" })`

2. **Create tasks from phases**: For each phase, create a task with proper dependencies
   - Use `TaskCreate` for each phase
   - Set up `blockedBy` relationships matching `depends_on`

3. **Spawn phase workers**: For phases with satisfied dependencies, spawn workers
   - Use `Task` with `team_name`, `name`, `subagent_type`, `run_in_background: true`
   - Worker prompt must include: phase goal, promise tag, skills content, budget
   - Workers report completion via `Teammate({ operation: "write", target_agent_id: "leader" })`

4. **Monitor and coordinate**:
   - Watch your inbox for completion messages
   - When a phase completes, check if blocked phases can now start
   - Spawn newly-unblocked phases immediately

5. **Run reviews** (if enabled): After each phase completes
   - Spawn review specialists in parallel
   - Collect findings
   - If any gating review fails, pause and report via callback

6. **Handle decomposition**: If a worker signals complexity
   - Receive `<spawn-subphase>` signals
   - Create sub-tasks with proper dependencies
   - Spawn sub-phase workers

7. **Report progress**: Call callbacks for Forge monitoring
   ```bash
   {callback_on_phase_complete} --phase {id} --status {status}
   ```

8. **Shutdown cleanly**: When all phases complete
   - `requestShutdown` all teammates
   - Wait for approvals
   - `cleanup` the team
   - Output final summary

## Worker Prompt Template

When spawning a phase worker, use this prompt structure:

```
You are executing phase {number}: {name}

## Goal
{phase_description}

## Skills
{skills_content}

## Rules
- Budget: {budget} iterations max
- Output `<promise>{promise}</promise>` ONLY when fully complete
- Use `<progress>N%</progress>` for intermediate status
- Use `<blocker>description</blocker>` if stuck
- Send completion message to leader when done

## When Complete
Teammate({ operation: "write", target_agent_id: "leader", value: "PHASE {id} COMPLETE" })
```

## Execution Order

Analyze the dependency graph and execute in waves:
- Wave 1: Phases with no dependencies (can run in parallel)
- Wave 2: Phases whose dependencies are all in Wave 1 (run when Wave 1 completes)
- Continue until all phases complete

Begin execution now.
```

---

## Monitoring & State Sync

While Claude Code runs the swarm, Forge actively monitors execution and keeps its state synchronized.

```
┌─────────────────────────────────────────────────────────────────┐
│                    FORGE MONITOR LOOP                            │
│                                                                  │
│   ┌─────────────┐     ┌─────────────┐     ┌─────────────┐       │
│   │   Callback  │     │   Inbox     │     │   File      │       │
│   │   Server    │     │   Watcher   │     │   Watcher   │       │
│   │   (HTTP)    │     │   (poll)    │     │   (notify)  │       │
│   └──────┬──────┘     └──────┬──────┘     └──────┬──────┘       │
│          │                   │                   │               │
│          └───────────────────┼───────────────────┘               │
│                              ▼                                   │
│                    ┌─────────────────┐                          │
│                    │  Event Router   │                          │
│                    └────────┬────────┘                          │
│                             │                                    │
│          ┌──────────────────┼──────────────────┐                │
│          ▼                  ▼                  ▼                │
│   ┌─────────────┐    ┌─────────────┐    ┌─────────────┐        │
│   │   State     │    │   Audit     │    │    UI       │        │
│   │   Updater   │    │   Logger    │    │   Display   │        │
│   └─────────────┘    └─────────────┘    └─────────────┘        │
└─────────────────────────────────────────────────────────────────┘
```

### Three Monitoring Channels

1. **Callback Server** - Forge starts a lightweight HTTP server on a random port
2. **Inbox Watcher** - Polls `~/.claude/teams/{team}/inboxes/leader.json` for messages
3. **File Watcher** - Monitors project directory for file changes

### State Synchronization

```rust
pub struct SwarmMonitor {
    team_name: String,
    callback_port: u16,
    state_manager: StateManager,
    audit_logger: AuditLogger,
}

impl SwarmMonitor {
    pub async fn run(&self) -> Result<SwarmResult> {
        let (tx, mut rx) = mpsc::channel(100);

        tokio::select! {
            _ = self.run_callback_server(tx.clone()) => {},
            _ = self.watch_inbox(tx.clone()) => {},
            _ = self.watch_files(tx.clone()) => {},
            result = self.process_events(&mut rx) => return result,
        }
    }
}
```

---

## Review Specialist Integration

After each phase completes, review specialists analyze the changes in parallel.

```
Phase Complete
       │
       ▼
┌──────────────────────────────────────────────────────────┐
│                  REVIEW DISPATCH                          │
│                                                          │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐        │
│  │  Security   │ │ Performance │ │Architecture │        │
│  │  Sentinel   │ │   Oracle    │ │ Strategist  │        │
│  │  gate: true │ │ gate: false │ │ gate: false │        │
│  └──────┬──────┘ └──────┬──────┘ └──────┬──────┘        │
│         └───────────────┼───────────────┘                │
│                         ▼                                │
│              ┌─────────────────┐                        │
│              │ Findings Aggreg │                        │
│              └────────┬────────┘                        │
│                       │                                  │
│         ┌─────────────┴─────────────┐                   │
│         ▼                           ▼                   │
│  ┌─────────────┐            ┌─────────────┐            │
│  │ Gate: PASS  │            │ Gate: FAIL  │            │
│  │ Continue    │            │ Apply       │            │
│  │             │            │ Resolution  │            │
│  └─────────────┘            └─────────────┘            │
└──────────────────────────────────────────────────────────┘
```

### Specialist Types

| Specialist | Focus |
|------------|-------|
| `security-sentinel` | SQL injection, XSS, auth bypass, secrets exposure |
| `performance-oracle` | N+1 queries, missing indexes, memory leaks |
| `architecture-strategist` | SOLID violations, coupling, layering |
| `simplicity-reviewer` | Over-engineering, premature abstraction |

### Review Output Format

```json
{
  "phase": "02",
  "reviewer": "security-sentinel",
  "verdict": "pass|warn|fail",
  "findings": [
    {
      "severity": "critical|warning|info",
      "file": "path/to/file.rs",
      "line": 42,
      "issue": "Description of the issue",
      "suggestion": "How to fix it"
    }
  ],
  "summary": "Overall assessment"
}
```

---

## LLM Arbiter

When a gating review fails, three resolution modes are available:

### Resolution Modes

1. **Manual** (`--review-mode manual`) - Always pause for user input
2. **Auto** (`--review-mode auto`) - Spawn fix agent, retry automatically
3. **Arbiter** (`--review-mode arbiter`) - LLM judges severity and decides

### Arbiter Decision Flow

```
Review Gate: FAIL
       │
       ▼
┌─────────────────┐
│ Arbiter Prompt  │
│                 │
│ Analyzes:       │
│ - Findings      │
│ - Risk profile  │
│ - Budget left   │
└────────┬────────┘
         │
   ┌─────┴─────┬─────────────┐
   ▼           ▼             ▼
PROCEED       FIX        ESCALATE
(continue)  (auto-fix)   (human)
```

### Arbiter Output

```json
{
  "decision": "PROCEED|FIX|ESCALATE",
  "reasoning": "Why this decision",
  "confidence": 0.0-1.0,
  "fix_instructions": "If FIX: specific instructions",
  "escalation_summary": "If ESCALATE: summary for human"
}
```

### Configuration

```toml
[swarm.reviews]
mode = "arbiter"
arbiter_confidence = 0.8
escalate_on = ["critical_security", "data_loss_risk"]
auto_proceed_on = ["style", "minor_performance"]
```

---

## Dynamic Decomposition

When a phase worker detects scope is larger than expected, decomposition is triggered.

### Trigger Conditions

- Worker emits `<blocker>` with complexity signal
- Iterations > 50% budget with progress < 30%
- Worker explicitly requests: `<request-decomposition/>`

### Decomposition Flow

```
Worker signals complexity
       │
       ▼
┌──────────────────┐
│ Decomposition    │
│ Agent analyzes:  │
│ - Original goal  │
│ - Work done      │
│ - Remaining work │
└────────┬─────────┘
         │
         ▼
┌──────────────────────────────────────┐
│ New Sub-Phase Structure              │
│                                      │
│ Phase 05 (paused)                    │
│   ├── 05.1: Google OAuth ──┐         │
│   ├── 05.2: GitHub OAuth ──┼─ ||     │
│   ├── 05.3: Auth0 OAuth  ──┘         │
│   └── 05.4: Unified handler          │
│            depends_on: [05.1-3]      │
└──────────────────────────────────────┘
```

### Decomposition Output

```json
{
  "analysis": "OAuth requires 3 separate provider integrations",
  "sub_phases": [
    {
      "number": "05.1",
      "name": "Google OAuth integration",
      "promise": "GOOGLE_OAUTH_COMPLETE",
      "budget": 5,
      "depends_on": [],
      "can_parallel": true
    }
  ],
  "integration_phase": {
    "number": "05.4",
    "name": "Unified OAuth callback handler",
    "promise": "OAUTH_UNIFIED_COMPLETE",
    "budget": 3,
    "depends_on": ["05.1", "05.2", "05.3"]
  }
}
```

---

## CLI Interface

### New Commands

```bash
forge swarm [OPTIONS]           # Parallel swarm execution
forge swarm status              # Monitor running swarm
forge swarm abort               # Gracefully stop swarm
```

### Full Flag Reference

```
forge swarm [OPTIONS]

EXECUTION:
    --from <PHASE>              Start from specific phase number
    --only <PHASES>             Run only specified phases
    --max-concurrent <N>        Maximum parallel workers [default: 4]
    --backend <TYPE>            auto, in-process, tmux, iterm2

REVIEWS:
    --review <SPECIALISTS>      security, performance, architecture, all
    --review-mode <MODE>        manual, auto, arbiter
    --max-fix-attempts <N>      Max auto-fix attempts [default: 2]
    --escalate-on <TYPES>       Always escalate these finding types
    --arbiter-confidence <N>    Min confidence threshold [default: 0.7]

DECOMPOSITION:
    --decompose                 Enable dynamic decomposition [default]
    --no-decompose              Disable decomposition
    --decompose-threshold <N>   Budget % to trigger [default: 50]

APPROVAL:
    --yes                       Auto-approve all prompts
    --permission-mode <MODE>    strict, standard, autonomous

BLUEPRINT:
    --dry-run                   Generate blueprint only
    --output <FILE>             Output blueprint to file
    --blueprint <FILE>          Use existing blueprint

MONITORING:
    --ui <MODE>                 full, minimal, json [default: full]
```

### Backwards Compatibility

```bash
forge run                    # Sequential (unchanged)
forge run --parallel         # Alias for 'forge swarm'
```

---

## Error Handling & Recovery

### Failure Taxonomy

| Category | Failure | Response |
|----------|---------|----------|
| Worker | Budget exhausted | Mark failed, continue others |
| Worker | Crashed | Respawn from checkpoint |
| Worker | Heartbeat timeout | Reassign task |
| Review | Timeout | Retry once, then skip |
| Review | Gating failure | Apply resolution mode |
| Coordination | Leader crashed | Forge detects, can resume |
| Coordination | Callback unreachable | Fallback to inbox polling |
| Infrastructure | API error | Exponential backoff |

### Checkpoint System

```json
{
  "phase": "05",
  "worker": "phase-05-worker",
  "iteration": 7,
  "timestamp": "2026-01-26T15:30:00Z",
  "progress_percent": 60,
  "files_changed": ["src/auth/oauth.rs"],
  "git_ref": "abc123f",
  "last_output_summary": "Implemented Google OAuth...",
  "context_size": 125000
}
```

### Recovery Commands

```bash
# Automatic resume detection
forge swarm

# Explicit resume
forge swarm --resume forge-run-20260126-150322

# Fresh start
forge swarm --fresh

# Reconcile state after crash
forge swarm --reconcile
```

### Graceful Shutdown

```
^C
Graceful shutdown initiated...
Signaling workers to complete current iteration...
Saving checkpoints...
Requesting teammate shutdowns...
Cleaning up team...

Progress saved. Resume with:
  forge swarm --resume forge-run-20260126-150322
```

---

## Implementation Plan

### Module Structure

```
src/
├── swarm/
│   ├── mod.rs
│   ├── blueprint.rs       # Blueprint generation
│   ├── analyzer.rs        # DAG analysis
│   ├── launcher.rs        # Claude Code spawning
│   ├── monitor.rs         # Callback server, watchers
│   ├── reconcile.rs       # State reconciliation
│   └── prompts.rs         # Prompt templates
│
├── review/
│   ├── mod.rs
│   ├── specialists.rs     # Specialist definitions
│   ├── arbiter.rs         # LLM decision maker
│   └── findings.rs        # Finding aggregation
│
├── checkpoint/
│   ├── mod.rs
│   ├── writer.rs          # Persistence
│   └── recovery.rs        # Recovery strategies
```

### Timeline

| Phase | Duration | Deliverables |
|-------|----------|--------------|
| 1. Foundation | Week 1-2 | Blueprint, DAG analyzer, CLI |
| 2. Core Swarm | Week 3-4 | Launcher, monitor, state sync |
| 3. Reviews | Week 5-6 | Specialists, arbiter |
| 4. Resilience | Week 7-8 | Checkpoints, recovery |
| 5. Decomposition | Week 9-10 | Dynamic splitting |
| 6. Polish | Week 11-12 | UI, docs, testing |

### Dependencies

```toml
[dependencies]
axum = "0.7"              # Callback HTTP server
tokio-stream = "0.1"      # Async file watching
notify = "6.0"            # File system notifications
petgraph = "0.6"          # DAG operations
```

### Estimated Scope

| Module | LOC |
|--------|-----|
| `swarm/` | ~1,500 |
| `review/` | ~800 |
| `checkpoint/` | ~500 |
| Config/CLI | ~300 |
| Tests | ~1,000 |
| **Total** | **~4,100** |

---

## Alternatives Considered

### Approach A: Native Parallel DAG + Hook-Based Swarms

Build parallel DAG execution natively in Rust, integrate swarms via hooks.

**Pros:**
- Faster (native scheduling)
- More control
- Less dependency on Claude Code internals

**Cons:**
- More code to maintain
- Two coordination mechanisms
- Doesn't leverage full swarm capability

**Why not chosen:** Higher implementation effort for less capability. Claude Code's swarm is battle-tested.

### Approach C: Forge as Swarm Backend

Forge manages team/task/inbox files, Claude instances are pure workers.

**Pros:**
- Full control
- No Claude Code dependency

**Cons:**
- Reimplements existing functionality
- Significant effort
- Missing swarm features (heartbeats, backends, etc.)

**Why not chosen:** Reimplementing well-tested infrastructure is wasteful.

---

## Open Questions

1. **Callback vs Polling:** Should we rely primarily on callbacks or treat them as optimization over polling?

2. **Budget Extension:** Should arbiter be able to extend phase budgets, or is that always human decision?

3. **Cross-Phase Reviews:** Should reviews only happen per-phase, or also at wave boundaries?

4. **Partial Completion:** If some phases in a wave fail, should we continue with non-dependent phases?

---

## Appendix: Swarm Primitives Reference

From Claude Code's swarm orchestration capabilities:

| Primitive | Description |
|-----------|-------------|
| Agent | Claude instance with tools |
| Team | Named group with leader + teammates |
| Task | Work item with status, owner, dependencies |
| Inbox | JSON message queue |
| Backend | Execution mode: in-process, tmux, iterm2 |

### Key TeammateTool Operations

- `spawnTeam` - Create team
- `write` - Message specific teammate
- `broadcast` - Message all
- `requestShutdown` / `approveShutdown` - Graceful exit
- `cleanup` - Remove team resources
