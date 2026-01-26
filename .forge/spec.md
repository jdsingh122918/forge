# Forge Swarm Orchestration Design

**Date:** 2026-01-26
**Status:** Draft
**Author:** Claude + User
**Approach:** Hybrid (Native DAG + Swarm Hooks)

---

## Executive Summary

This document describes the design for integrating parallel execution and swarm orchestration into Forge. The chosen approach is a **hybrid architecture**:

- **Native Rust DAG scheduler** for phase-level parallelism (fast, predictable, testable)
- **Swarm hooks** that delegate to Claude Code's TeammateTool for within-phase complexity

This gives us the best of both worlds: disciplined orchestration where Forge excels, and sophisticated agent coordination where Claude Code excels.

---

## Table of Contents

1. [Goals & Non-Goals](#goals--non-goals)
2. [Architecture Overview](#architecture-overview)
3. [Native DAG Scheduler](#native-dag-scheduler)
4. [Swarm Hook System](#swarm-hook-system)
5. [Review Specialist Integration](#review-specialist-integration)
6. [LLM Arbiter](#llm-arbiter)
7. [Dynamic Decomposition](#dynamic-decomposition)
8. [CLI Interface](#cli-interface)
9. [Error Handling & Recovery](#error-handling--recovery)
10. [Implementation Plan](#implementation-plan)

---

## Goals & Non-Goals

### Goals

1. **Faster execution** - Run independent phases in parallel via native DAG scheduling
2. **Better quality** - Swarm-based parallel review specialists gate phase completion
3. **Smarter decomposition** - Automatically split complex phases using Claude agents
4. **Autonomous operation** - LLM arbiter can resolve review failures without human intervention
5. **Resilience** - Recover gracefully from crashes via checkpointing
6. **Predictable behavior** - Native scheduling eliminates LLM latency variance in critical path
7. **Backwards compatibility** - Existing `forge run` continues to work

### Non-Goals

- Replacing Claude Code's swarm primitives with native implementations
- Supporting non-Claude LLM backends (initially)
- Real-time collaboration between human and swarm
- Distributed execution across multiple machines

---

## Architecture Overview

The hybrid architecture splits responsibilities clearly:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                            FORGE (Native Rust)                           │
│                                                                          │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │                        DAG SCHEDULER                                │ │
│  │                                                                     │ │
│  │   phases.json ──▶ [Build Graph] ──▶ [Compute Waves] ──▶ [Dispatch] │ │
│  │                                                                     │ │
│  │   Wave 1: [01]──────────────────────────────────────────────────┐  │ │
│  │   Wave 2: [02]─────┬─────[03]─────┬─────[06]────────────────────┤  │ │
│  │   Wave 3: [04]─────┴─────[05*]────┴─────[07]────────────────────┤  │ │
│  │   Wave 4: [08]──────────────────────────────────────────────────┘  │ │
│  │                        (* = swarm-enabled)                          │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                    │                                     │
│                    ┌───────────────┼───────────────┐                    │
│                    ▼               ▼               ▼                    │
│             ┌───────────┐   ┌───────────┐   ┌───────────┐              │
│             │  Worker   │   │  Worker   │   │  Swarm    │              │
│             │ (Claude)  │   │ (Claude)  │   │  Hook     │              │
│             └───────────┘   └───────────┘   └─────┬─────┘              │
│                                                   │                     │
└───────────────────────────────────────────────────┼─────────────────────┘
                                                    │
                    ┌───────────────────────────────┘
                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         CLAUDE CODE SWARM                                │
│                                                                          │
│   Leader Agent                                                          │
│      ├── Review Specialists (security, performance, architecture)       │
│      ├── Decomposition Agent (splits complex work)                      │
│      └── Task Workers (parallel execution within phase)                 │
│                                                                          │
│   Communicates via: TeammateTool, TaskCreate/Update, Inboxes            │
└─────────────────────────────────────────────────────────────────────────┘
```

### Responsibility Split

| Responsibility | Forge (Native) | Claude Code (Swarm) |
|----------------|----------------|---------------------|
| Phase-level parallelism | ✓ | |
| Dependency ordering | ✓ | |
| Progress tracking | ✓ | |
| State persistence | ✓ | |
| Error recovery | ✓ | |
| Agent coordination | | ✓ |
| Task decomposition | | ✓ |
| Review orchestration | | ✓ |
| Inter-agent messaging | | ✓ |

---

## Native DAG Scheduler

### Core Data Structures

```rust
// src/dag/mod.rs

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::algo::toposort;
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

/// The DAG scheduler manages parallel phase execution
pub struct DagScheduler {
    /// Directed graph of phase dependencies
    graph: DiGraph<PhaseNode, ()>,
    /// Map from phase number to graph node index
    node_map: HashMap<String, NodeIndex>,
    /// Current execution state
    state: DagState,
    /// Configuration
    config: DagConfig,
}

/// A node in the phase DAG
pub struct PhaseNode {
    pub phase: Phase,
    pub status: PhaseStatus,
    pub result: Option<PhaseResult>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhaseStatus {
    Pending,
    Blocked { waiting_on: Vec<String> },
    Ready,
    Running { started_at: Instant },
    Completed { iterations: u32 },
    Failed { error: String },
    Skipped,
}

pub struct DagConfig {
    /// Maximum phases to run in parallel
    pub max_parallel: usize,
    /// Stop all phases on first failure
    pub fail_fast: bool,
    /// Enable swarm hooks for marked phases
    pub swarm_enabled: bool,
    /// Backend for swarm execution
    pub swarm_backend: SwarmBackend,
}

#[derive(Debug, Clone)]
pub enum SwarmBackend {
    Auto,
    InProcess,
    Tmux,
    Iterm2,
}
```

### DAG Construction

```rust
// src/dag/builder.rs

impl DagScheduler {
    /// Build DAG from phases.json
    pub fn from_phases(phases: &[Phase], config: DagConfig) -> Result<Self> {
        let mut graph = DiGraph::new();
        let mut node_map = HashMap::new();

        // Add all phases as nodes
        for phase in phases {
            let node = PhaseNode {
                phase: phase.clone(),
                status: PhaseStatus::Pending,
                result: None,
            };
            let idx = graph.add_node(node);
            node_map.insert(phase.number.clone(), idx);
        }

        // Add dependency edges
        for phase in phases {
            let to_idx = node_map[&phase.number];
            for dep in &phase.depends_on {
                let from_idx = node_map.get(dep)
                    .ok_or_else(|| anyhow!("Unknown dependency: {}", dep))?;
                graph.add_edge(*from_idx, to_idx, ());
            }
        }

        // Verify no cycles
        toposort(&graph, None)
            .map_err(|_| anyhow!("Cycle detected in phase dependencies"))?;

        Ok(Self {
            graph,
            node_map,
            state: DagState::Idle,
            config,
        })
    }

    /// Compute execution waves (phases that can run in parallel)
    pub fn compute_waves(&self) -> Vec<Vec<String>> {
        let mut waves = Vec::new();
        let mut completed: HashSet<String> = HashSet::new();

        loop {
            // Find all phases whose dependencies are satisfied
            let ready: Vec<String> = self.graph.node_indices()
                .filter_map(|idx| {
                    let node = &self.graph[idx];
                    let phase_num = &node.phase.number;

                    if completed.contains(phase_num) {
                        return None;
                    }

                    let deps_satisfied = node.phase.depends_on.iter()
                        .all(|dep| completed.contains(dep));

                    if deps_satisfied {
                        Some(phase_num.clone())
                    } else {
                        None
                    }
                })
                .collect();

            if ready.is_empty() {
                break;
            }

            completed.extend(ready.iter().cloned());
            waves.push(ready);
        }

        waves
    }
}
```

### Parallel Executor

```rust
// src/dag/executor.rs

impl DagScheduler {
    /// Execute all phases respecting dependencies and parallelism limits
    pub async fn execute(&mut self, runner: &ClaudeRunner) -> Result<DagResult> {
        self.state = DagState::Running;

        let (tx, mut rx) = mpsc::channel::<PhaseEvent>(100);
        let semaphore = Arc::new(Semaphore::new(self.config.max_parallel));

        // Track active tasks
        let mut active_tasks: HashMap<String, JoinHandle<PhaseResult>> = HashMap::new();

        loop {
            // Find phases ready to run
            let ready = self.get_ready_phases();

            // Spawn tasks for ready phases (respecting semaphore)
            for phase_num in ready {
                let permit = semaphore.clone().acquire_owned().await?;
                let phase = self.get_phase(&phase_num).clone();
                let runner = runner.clone();
                let tx = tx.clone();
                let swarm_enabled = self.is_swarm_phase(&phase_num);

                let handle = tokio::spawn(async move {
                    let _permit = permit; // Hold until complete

                    let result = if swarm_enabled {
                        execute_swarm_phase(&phase, &runner).await
                    } else {
                        execute_standard_phase(&phase, &runner).await
                    };

                    tx.send(PhaseEvent::Completed {
                        phase: phase.number.clone(),
                        result: result.clone(),
                    }).await.ok();

                    result
                });

                active_tasks.insert(phase_num.clone(), handle);
                self.mark_running(&phase_num);
            }

            // Wait for any phase to complete
            if active_tasks.is_empty() && self.all_complete() {
                break;
            }

            match rx.recv().await {
                Some(PhaseEvent::Completed { phase, result }) => {
                    active_tasks.remove(&phase);
                    self.record_result(&phase, result)?;

                    if self.config.fail_fast && result.is_err() {
                        self.cancel_all(&mut active_tasks).await;
                        break;
                    }
                }
                Some(PhaseEvent::Progress { phase, percent }) => {
                    self.update_progress(&phase, percent);
                }
                None => break,
            }
        }

        self.state = DagState::Completed;
        Ok(self.build_result())
    }

    fn get_ready_phases(&self) -> Vec<String> {
        self.graph.node_indices()
            .filter_map(|idx| {
                let node = &self.graph[idx];

                if !matches!(node.status, PhaseStatus::Pending) {
                    return None;
                }

                let deps_done = node.phase.depends_on.iter().all(|dep| {
                    matches!(
                        self.get_status(dep),
                        PhaseStatus::Completed { .. } | PhaseStatus::Skipped
                    )
                });

                if deps_done {
                    Some(node.phase.number.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}
```

---

## Swarm Hook System

### New Hook Event and Type

```rust
// src/hooks/types.rs (extensions)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    // Existing events
    PrePhase,
    PostPhase,
    PreIteration,
    PostIteration,
    OnFailure,
    OnApproval,
    // New swarm events
    SwarmDispatch,      // When a phase needs swarm execution
    SwarmTaskComplete,  // When a swarm task finishes
    SwarmReview,        // When reviews are triggered
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookType {
    Command,
    Prompt,
    Swarm,  // New type
}
```

### Swarm Context

```rust
// src/swarm/context.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmContext {
    /// Phase being executed
    pub phase: PhaseInfo,
    /// Tasks to distribute (if decomposed)
    pub tasks: Vec<SwarmTask>,
    /// Execution strategy
    pub strategy: SwarmStrategy,
    /// Review configuration
    pub reviews: Option<ReviewConfig>,
    /// Callback endpoint for progress
    pub callback_url: String,
    /// Working directory
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTask {
    pub id: String,
    pub name: String,
    pub description: String,
    pub files: Vec<String>,
    pub depends_on: Vec<String>,
    pub budget: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmStrategy {
    /// All tasks run simultaneously
    Parallel,
    /// Tasks run one at a time
    Sequential,
    /// Dependency-ordered waves
    WavePipeline,
    /// Let the swarm leader decide
    Adaptive,
}
```

### Swarm Executor

```rust
// src/swarm/executor.rs

pub struct SwarmExecutor {
    claude_cmd: String,
    backend: SwarmBackend,
    callback_server: CallbackServer,
}

impl SwarmExecutor {
    pub async fn execute(&self, context: SwarmContext) -> Result<SwarmResult> {
        // 1. Start callback server for progress updates
        let callback_url = self.callback_server.start().await?;

        // 2. Generate the orchestration prompt
        let prompt = self.build_orchestration_prompt(&context, &callback_url)?;

        // 3. Invoke Claude Code with swarm capabilities
        let mut cmd = Command::new(&self.claude_cmd);
        cmd.arg("--print")
           .arg("--dangerously-skip-permissions")
           .arg("--output-format").arg("stream-json")
           .current_dir(&context.working_dir);

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // 4. Send prompt via stdin
        child.stdin.take().unwrap()
            .write_all(prompt.as_bytes()).await?;

        // 5. Monitor execution via callbacks and stdout
        let result = self.monitor_execution(child, &context).await?;

        // 6. Cleanup
        self.callback_server.stop().await?;

        Ok(result)
    }

    fn build_orchestration_prompt(
        &self,
        context: &SwarmContext,
        callback_url: &str,
    ) -> Result<String> {
        Ok(format!(r#"
# Forge Swarm Orchestrator

You are coordinating a swarm for Forge phase {phase_num}: {phase_name}

## Context
{context_json}

## Your Responsibilities

1. **Create team**: `Teammate({{ operation: "spawnTeam", team_name: "forge-{phase_num}" }})`

2. **Spawn workers** for each task:
   ```javascript
   Task({{
     team_name: "forge-{phase_num}",
     name: "task-{{id}}",
     subagent_type: "general-purpose",
     prompt: "Execute task: {{description}}",
     run_in_background: true
   }})
   ```

3. **Monitor completion** via inbox messages

4. **Report progress** to Forge:
   ```bash
   curl -X POST {callback_url}/progress -d '{{"task": "...", "status": "..."}}'
   ```

5. **Run reviews** if configured (spawn review specialists in parallel)

6. **Shutdown cleanly** when all tasks complete

## Completion Signal

When finished, output:
```xml
<swarm_complete>
{{"success": true, "tasks_completed": [...], "reviews": [...]}}
</swarm_complete>
```

Begin now.
"#,
            phase_num = context.phase.number,
            phase_name = context.phase.name,
            context_json = serde_json::to_string_pretty(&context)?,
            callback_url = callback_url,
        ))
    }
}
```

### Phase Configuration for Swarm

```rust
// Extension to src/phase.rs

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Phase {
    // ... existing fields ...

    /// Enable swarm execution for this phase
    #[serde(default)]
    pub swarm: Option<SwarmConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwarmConfig {
    /// Execution strategy
    pub strategy: SwarmStrategy,
    /// Maximum agents to spawn
    pub max_agents: u32,
    /// Pre-defined task breakdown (optional)
    pub tasks: Option<Vec<SwarmTask>>,
    /// Enable review specialists
    pub reviews: Option<Vec<ReviewSpecialist>>,
}
```

### Configuration

```toml
# forge.toml

[swarm]
enabled = true
backend = "auto"                    # auto, in-process, tmux, iterm2
default_strategy = "adaptive"
max_agents = 5

[swarm.reviews]
enabled = true
specialists = ["security", "performance"]
mode = "arbiter"                    # manual, auto, arbiter

# Per-phase swarm configuration
[phases.overrides."*-complex"]
swarm = { strategy = "parallel", max_agents = 4 }

[phases.overrides."*-refactor"]
swarm = { strategy = "wave_pipeline", reviews = ["architecture"] }
```

---

## Review Specialist Integration

### Review Flow

```
Phase Complete (or Swarm Complete)
       │
       ▼
┌──────────────────────────────────────────────────────────┐
│  SWARM REVIEW DISPATCH (within Claude Code swarm)        │
│                                                          │
│  Leader spawns review agents in parallel:                │
│                                                          │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐        │
│  │  Security   │ │ Performance │ │Architecture │        │
│  │  Sentinel   │ │   Oracle    │ │ Strategist  │        │
│  │  gate: true │ │ gate: false │ │ gate: true  │        │
│  └──────┬──────┘ └──────┬──────┘ └──────┬──────┘        │
│         └───────────────┼───────────────┘                │
│                         ▼                                │
│              ┌─────────────────┐                        │
│              │   Aggregate     │                        │
│              │   Findings      │                        │
│              └────────┬────────┘                        │
│                       │                                  │
│         ┌─────────────┴─────────────┐                   │
│         ▼                           ▼                   │
│     Gate PASS                   Gate FAIL               │
│     (continue)                  (resolve)               │
└──────────────────────────────────────────────────────────┘
```

### Review Specialist Types

```rust
// src/review/specialists.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSpecialist {
    pub specialist_type: SpecialistType,
    pub gate: bool,
    pub focus_areas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialistType {
    SecuritySentinel,
    PerformanceOracle,
    ArchitectureStrategist,
    SimplicityReviewer,
    Custom(String),
}

impl SpecialistType {
    pub fn focus_areas(&self) -> Vec<&str> {
        match self {
            Self::SecuritySentinel => vec![
                "SQL injection", "XSS", "auth bypass",
                "secrets exposure", "input validation"
            ],
            Self::PerformanceOracle => vec![
                "N+1 queries", "missing indexes",
                "memory leaks", "algorithmic complexity"
            ],
            Self::ArchitectureStrategist => vec![
                "SOLID violations", "coupling",
                "layering", "separation of concerns"
            ],
            Self::SimplicityReviewer => vec![
                "over-engineering", "premature abstraction",
                "YAGNI violations", "unnecessary complexity"
            ],
            Self::Custom(_) => vec![],
        }
    }
}
```

### Review Output Format

```json
{
  "phase": "05",
  "reviewer": "security-sentinel",
  "verdict": "warn",
  "findings": [
    {
      "severity": "warning",
      "file": "src/auth/oauth.rs",
      "line": 142,
      "issue": "Token stored in localStorage is vulnerable to XSS",
      "suggestion": "Use httpOnly cookies instead"
    }
  ],
  "summary": "One medium-severity finding related to token storage"
}
```

---

## LLM Arbiter

When a gating review fails, three resolution modes are available:

### Resolution Modes

```rust
// src/review/arbiter.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionMode {
    /// Always pause for user input
    Manual,
    /// Attempt auto-fix, retry up to N times
    Auto { max_attempts: u32 },
    /// LLM decides based on severity and context
    Arbiter {
        model: String,
        confidence_threshold: f64,
        escalate_on: Vec<String>,
        auto_proceed_on: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbiterDecision {
    pub decision: ArbiterVerdict,
    pub reasoning: String,
    pub confidence: f64,
    pub fix_instructions: Option<String>,
    pub escalation_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ArbiterVerdict {
    Proceed,   // Continue despite findings
    Fix,       // Spawn fix agent, retry
    Escalate,  // Require human decision
}
```

### Arbiter Prompt

```markdown
# Review Arbiter

You are deciding how to handle review findings that would block progress.

## Context
- Phase: {phase_id} - {phase_name}
- Budget used: {iterations_used}/{budget}

## Blocking Findings
```json
{findings_json}
```

## Decision Criteria

**PROCEED** when:
- Style issues only
- Minor warnings
- False positives
- Acceptable trade-offs for MVP

**FIX** when:
- Clear fix path exists
- Security/correctness issues
- Within remaining budget

**ESCALATE** when:
- Architectural concerns
- Ambiguous risk
- Out of budget
- Policy decisions needed

## Output
```json
{
  "decision": "PROCEED|FIX|ESCALATE",
  "reasoning": "...",
  "confidence": 0.0-1.0,
  "fix_instructions": "if FIX",
  "escalation_summary": "if ESCALATE"
}
```
```

---

## Dynamic Decomposition

### Trigger Conditions

- Worker emits `<blocker>` with complexity signal
- Iterations > 50% budget with progress < 30%
- Worker explicitly requests: `<request-decomposition/>`

### Decomposition Agent

The swarm leader spawns a decomposition agent when triggered:

```rust
// src/swarm/decomposition.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionResult {
    pub analysis: String,
    pub sub_phases: Vec<SubPhaseSpec>,
    pub integration_phase: Option<SubPhaseSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubPhaseSpec {
    pub number: String,
    pub name: String,
    pub promise: String,
    pub budget: u32,
    pub depends_on: Vec<String>,
    pub can_parallel: bool,
    pub reasoning: String,
}
```

### Example Decomposition

```
Phase 05: OAuth Integration (budget: 20)
         │
         │ Worker: "This requires 3 separate provider integrations"
         ▼
┌────────────────────────────────────────┐
│ Decomposition Agent analyzes...        │
│                                        │
│ Output:                                │
│ ├── 05.1: Google OAuth (budget: 5) ─┐  │
│ ├── 05.2: GitHub OAuth (budget: 5) ─┼─ parallel
│ ├── 05.3: Auth0 OAuth  (budget: 5) ─┘  │
│ └── 05.4: Unified handler (budget: 3)  │
│           depends_on: [05.1-3]         │
└────────────────────────────────────────┘
```

---

## CLI Interface

### Commands

```bash
# Existing (unchanged)
forge run                         # Sequential execution
forge run --phase 05              # Single phase

# New parallel/swarm commands
forge swarm                       # Parallel DAG execution
forge swarm --from 05             # Start from phase
forge swarm --max-parallel 3      # Limit concurrency
forge swarm --review security,performance
forge swarm --review-mode arbiter --yes

# Monitoring
forge swarm status                # Show running swarm
forge swarm abort                 # Graceful stop

# Backwards compatibility
forge run --parallel              # Alias for 'forge swarm'
```

### Full Flag Reference

```
forge swarm [OPTIONS]

EXECUTION:
    --from <PHASE>              Start from specific phase
    --only <PHASES>             Run only these phases (comma-separated)
    --max-parallel <N>          Max concurrent phases [default: 4]
    --backend <TYPE>            auto, in-process, tmux, iterm2

REVIEWS:
    --review <SPECIALISTS>      security, performance, architecture, all
    --review-mode <MODE>        manual, auto, arbiter [default: manual]
    --max-fix-attempts <N>      Auto-fix attempts [default: 2]
    --escalate-on <TYPES>       Always escalate these findings
    --arbiter-confidence <N>    Min confidence [default: 0.7]

DECOMPOSITION:
    --decompose                 Enable decomposition [default]
    --no-decompose              Disable decomposition
    --decompose-threshold <N>   Budget % trigger [default: 50]

APPROVAL:
    --yes                       Auto-approve all prompts
    --permission-mode <MODE>    strict, standard, autonomous

MONITORING:
    --ui <MODE>                 full, minimal, json [default: full]
```

### Example Session

```bash
$ forge swarm --review security --max-parallel 3

Analyzing phase dependencies...
  22 phases, 8 execution waves

Wave 1: [01] ████████████ 100%  (3 iterations)

Wave 2: [02] ████████░░░░  67%
        [03] ████████████ 100%
        [06] ████████████ 100%

Reviews for [03]:
  ✓ security-sentinel: PASS

Wave 2: [02] ████████████ 100%  (10 iterations)

Reviews for [02]:
  ⚠ security-sentinel: WARN (1 finding)
    └─ src/db/queries.rs:42 - Consider parameterized query

Continuing (non-gating)...

Wave 3: [04] ████░░░░░░░░  33%
        [05*] Starting swarm...
              └─ Spawning 3 agents for OAuth integration
        [07] ████████████ 100%
```

---

## Error Handling & Recovery

### Failure Taxonomy

| Category | Failure | Detection | Recovery |
|----------|---------|-----------|----------|
| Phase | Budget exhausted | Native | Mark failed, continue DAG |
| Phase | Promise not found | Native | Retry with hints |
| Swarm | Agent crash | Callback timeout | Respawn from checkpoint |
| Swarm | Leader crash | Process monitor | Resume with state |
| Review | Timeout | Native | Retry once, then skip |
| Review | Gate failure | Native | Apply resolution mode |
| Infra | Claude API error | HTTP status | Exponential backoff |

### Checkpoint System

```rust
// src/checkpoint/mod.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub phase: String,
    pub iteration: u32,
    pub timestamp: DateTime<Utc>,
    pub progress_percent: u32,
    pub files_changed: Vec<String>,
    pub git_ref: String,
    pub context_summary: String,
}

impl Checkpoint {
    pub fn save(&self, checkpoint_dir: &Path) -> Result<()> {
        let path = checkpoint_dir.join(format!("{}.json", self.phase));
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    pub fn load(checkpoint_dir: &Path, phase: &str) -> Result<Option<Self>> {
        let path = checkpoint_dir.join(format!("{}.json", phase));
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        Ok(Some(serde_json::from_str(&content)?))
    }
}
```

### Recovery Flow

```bash
# Automatic resume detection
$ forge swarm
Detected incomplete run: forge-run-20260126-150322
  Progress: 14/22 phases
  Checkpoints: 3 phases resumable

Resume? [Y/n] y

Recovering...
  ✓ Loaded checkpoints
  ✓ Reconciled state
  ✓ Respawning 3 phases

Continuing from Wave 4...
```

---

## Implementation Plan

### Module Structure

```
src/
├── main.rs                      # Add swarm subcommand
├── lib.rs                       # Export new modules
│
├── dag/                         # NEW: DAG scheduler
│   ├── mod.rs
│   ├── builder.rs               # Build graph from phases
│   ├── scheduler.rs             # Core scheduling logic
│   ├── executor.rs              # Parallel execution
│   └── state.rs                 # Execution state tracking
│
├── swarm/                       # NEW: Swarm integration
│   ├── mod.rs
│   ├── executor.rs              # Swarm hook executor
│   ├── context.rs               # SwarmContext types
│   ├── callback.rs              # HTTP callback server
│   └── prompts.rs               # Orchestration prompts
│
├── review/                      # NEW: Review system
│   ├── mod.rs
│   ├── specialists.rs           # Specialist definitions
│   ├── arbiter.rs               # LLM decision maker
│   └── findings.rs              # Finding types
│
├── checkpoint/                  # NEW: Checkpointing
│   ├── mod.rs
│   ├── writer.rs
│   └── recovery.rs
│
├── hooks/
│   ├── types.rs                 # MODIFY: Add SwarmDispatch event
│   └── executor.rs              # MODIFY: Add swarm execution
│
├── phase.rs                     # MODIFY: Add swarm config
└── forge_config.rs              # MODIFY: Add swarm section
```

### Dependencies

```toml
# Cargo.toml additions

[dependencies]
petgraph = "0.6"          # DAG operations
axum = "0.7"              # Callback HTTP server
notify = "6.0"            # File watching
tokio-stream = "0.1"      # Async streaming
```

### Timeline

| Phase | Duration | Deliverables |
|-------|----------|--------------|
| 1. DAG Core | Week 1-2 | Graph builder, wave computation, basic executor |
| 2. Parallel Execution | Week 3-4 | Tokio-based parallel dispatch, state tracking |
| 3. Swarm Hooks | Week 5-6 | SwarmContext, executor, callback server |
| 4. Reviews | Week 7-8 | Specialists, findings, arbiter |
| 5. Checkpoints | Week 9-10 | Persistence, recovery, reconciliation |
| 6. Polish | Week 11-12 | CLI, UI, documentation, testing |

### Estimated Scope

| Module | LOC |
|--------|-----|
| `dag/` | ~800 |
| `swarm/` | ~700 |
| `review/` | ~600 |
| `checkpoint/` | ~400 |
| Config/CLI | ~300 |
| Tests | ~1,000 |
| **Total** | **~3,800** |

---

## Open Questions

1. **Swarm backend selection**: Should we auto-detect or require explicit configuration?

2. **Cross-wave reviews**: Should reviews happen per-phase, per-wave, or configurable?

3. **Budget extension**: Can arbiter extend budgets, or is that always human decision?

4. **Partial wave failure**: If some phases in a wave fail, continue with non-dependent phases?

5. **Swarm task granularity**: How fine-grained should decomposition be? File-level? Function-level?

---

## Summary

The hybrid approach gives us:

- **Fast, predictable scheduling** via native Rust DAG
- **Sophisticated agent coordination** via Claude Code swarms
- **Quality gates** via parallel review specialists
- **Autonomous operation** via LLM arbiter
- **Resilience** via checkpointing and recovery
- **Clear separation** of concerns between Forge and Claude Code

This architecture plays to the strengths of both systems while maintaining Forge's philosophy of disciplined, observable orchestration.
