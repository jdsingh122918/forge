# ADR 001: DAG Scheduler for Parallel Phase Execution

**Status:** Accepted
**Date:** 2026-01-26

---

## Context

Forge's original design executed phases **sequentially** — one phase at a time, waiting for the previous to complete. This was simple but left capability untapped:

- Independent phases (e.g. "write tests" and "write docs") had no data dependency, yet neither could start until the other finished.
- Large projects with 10–20 phases could take hours of wall-clock time even when work was parallelisable.

## Decision

We replaced sequential execution with a **DAG (Directed Acyclic Graph) scheduler** that:

1. Parses each phase's `dependencies` array from `phases.json` at startup.
2. Builds a petgraph `DiGraph` where each node is a phase and each edge is a dependency.
3. At runtime, computes **execution waves** — sets of phases whose dependencies are all satisfied — and dispatches all phases in a wave concurrently up to `max_parallel` (default 4).
4. As each phase completes, recomputes the ready set and dispatches the next wave.

Implemented in `src/dag/scheduler.rs` (wave computation) and `src/dag/executor.rs` (async dispatch).

## Alternatives Considered

| Alternative | Reason Rejected |
|-------------|-----------------|
| Keep sequential execution | Too slow for large projects |
| External workflow engine (Temporal) | Heavy external dependency |
| Topological sort only (strict ordering) | Doesn't exploit parallelism between independent branches |

## Consequences

**Positive:**
- Wall-clock time drops dramatically for projects with independent branches.
- DAG naturally detects cycles at load time with a clear error.
- `forge swarm` is now a thin wrapper around the same DAG executor.

**Negative:**
- `phases.json` requires an explicit `dependencies` list; omitting a dependency causes file races.
- `max_parallel` must be tuned to Claude API rate limits; too high causes 429 errors.

## Configuration

```toml
# .forge/forge.toml
[swarm]
max_parallel = 4
fail_fast = false
```

## Related

- `src/dag/scheduler.rs` — wave computation, `PhaseStatus` state machine
- `src/dag/executor.rs` — async task dispatch and result collection
- `src/swarm/executor.rs` — swarm-mode wrapper using the DAG executor
