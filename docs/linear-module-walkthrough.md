# Linear Module Walkthrough

This is a read-in-order map of the production modules in the Forge workspace.
It complements `docs/context-graph.md` and `docs/context-graph-file-deps.md`
by giving you one linear path through the code instead of a graph.

The order below follows the current user-facing flow first:

1. CLI entrypoints and command routing
2. Config, spec, and phase generation
3. Sequential orchestration
4. Parallel execution, reviews, council, and swarm
5. Factory backend and React UI
6. Runtime-platform workspace crates

Test-only modules are listed in an appendix so the main walkthrough stays on
the runtime path.

## 1. Start At The Crate Surface

- `src/lib.rs`: The root library surface re-exports every major subsystem so the CLI and tests can import one crate.
- `src/main.rs`: The `forge` executable parses CLI flags, initializes telemetry, and dispatches subcommands into `src/cmd`.
- `src/cmd/mod.rs`: The command router maps each Clap subcommand onto a dedicated handler module.
- `src/cmd/project.rs`: Owns the project-bootstrap path for `init`, `interview`, `generate`, and `implement`.
- `src/cmd/run.rs`: Owns the sequential execution entrypoints for `forge run` and `forge phase`.
- `src/cmd/phase.rs`: Owns the read-only phase commands such as `list`, `status`, `reset`, and `audit`.
- `src/cmd/swarm.rs`: Owns the DAG and parallel-execution CLI surface.
- `src/cmd/factory.rs`: Launches the Factory server and UI shell.
- `src/cmd/compact.rs`: Exposes manual context-compaction control from the CLI.
- `src/cmd/patterns.rs`: Exposes pattern learning, listing, comparison, and recommendation commands.
- `src/cmd/config.rs`: Exposes config inspection, validation, and initialization commands.
- `src/cmd/skills.rs`: Exposes skill listing and CRUD around `.forge/skills`.
- `src/cmd/update.rs`: Exposes self-update and version-check commands.
- `src/cmd/autoresearch/mod.rs`: Namespaces the benchmark-oriented autoresearch commands.
- `src/cmd/autoresearch/runner.rs`: Runs a single benchmark or benchmark batch through the specialist pipeline.
- `src/cmd/autoresearch/loop_runner.rs`: Repeats benchmark runs in a loop for experiment-style evaluation.
- `src/cmd/autoresearch/experiment.rs`: Defines the higher-level experiment execution shape and orchestration helpers.
- `src/cmd/autoresearch/judge.rs`: Runs the judge stage that evaluates benchmark outputs.
- `src/cmd/autoresearch/scorer.rs`: Scores outputs against expected benchmark results.
- `src/cmd/autoresearch/results.rs`: Writes and formats persisted benchmark results.
- `src/cmd/autoresearch/budget.rs`: Tracks and reports budget-oriented benchmark summaries.
- `src/cmd/autoresearch/git_ops.rs`: Handles the git-side helpers needed by autoresearch experiments.

## 2. Understand Config, Spec, And Phase Data

- `src/config.rs`: Builds the runtime `Config` that resolves project paths, spec files, logs, audit files, and CLI overrides.
- `src/forge_config.rs`: Loads layered `.forge/forge.toml` config, including permission mode, reviews, decomposition, and council settings.
- `src/phase.rs`: Defines the `Phase` and `SubPhase` schemas and loads `phases.json`.
- `src/init/mod.rs`: Creates the `.forge` scaffold and bootstraps a new project workspace.
- `src/interview/mod.rs`: Drives the interactive spec interview that turns user answers into a project plan.
- `src/generate/mod.rs`: Converts a spec into the phase plan that the orchestrator executes.
- `src/implement/mod.rs`: Wraps design-doc implementation into a higher-level TDD-oriented workflow.
- `src/implement/extract.rs`: Extracts structured sections and implementation details from design docs.
- `src/implement/spec_gen.rs`: Generates implementation specs and phases from extracted design data.
- `src/implement/types.rs`: Holds shared data structures for the `forge implement` flow.
- `src/patterns/mod.rs`: Groups the pattern-learning and budget-intelligence modules behind one namespace.
- `src/patterns/learning.rs`: Learns reusable patterns from completed projects and persists them globally.
- `src/patterns/budget_suggester.rs`: Matches saved patterns to new work and suggests budgets or skills.
- `src/patterns/stats_aggregator.rs`: Aggregates cross-pattern statistics for reporting and recommendations.
- `src/skills/mod.rs`: Loads reusable skill markdown and formats it for prompt injection.
- `src/update_check.rs`: Reads and writes update-check config and cache data in the user's global Forge directory.
- `src/telemetry.rs`: Initializes tracing, file logs, and stderr logging format.
- `src/errors.rs`: Defines the typed error hierarchy used by orchestrator, phase, and Factory code.
- `src/util.rs`: Holds small shared helpers, such as extracting embedded JSON from model output.

## 3. Follow The Sequential Orchestrator

- `src/orchestrator/mod.rs`: Defines the sequential orchestration namespace and re-exports the runner, state, and review bridge.
- `src/orchestrator/state.rs`: Persists append-only phase progress so `forge run` can recover and skip completed work.
- `src/orchestrator/review_integration.rs`: Bridges a completed phase into the review subsystem and interprets the result.
- `src/orchestrator/runner.rs`: Runs the per-phase iteration loop, writes prompts, spawns Claude, parses stream output, and checks promise tags.
- `src/gates/mod.rs`: Implements phase and iteration approval logic for standard, autonomous, and readonly modes.
- `src/stream/mod.rs`: Parses Claude `stream-json` output into text, tool-use, result, and session events.
- `src/signals/mod.rs`: Namespaces the progress-signal parser and signal data types.
- `src/signals/parser.rs`: Extracts `<progress>`, `<blocker>`, `<pivot>`, and `<spawn-subphase>` tags from model output.
- `src/signals/types.rs`: Defines the typed signal payloads used across orchestration and UI.
- `src/hooks/mod.rs`: Defines the hook subsystem boundary and shared exports.
- `src/hooks/config.rs`: Loads hook definitions from hook config files and resolves their matching rules.
- `src/hooks/types.rs`: Defines hook events, actions, context, and results.
- `src/hooks/executor.rs`: Executes command hooks, prompt hooks, and swarm hooks.
- `src/hooks/manager.rs`: Matches hooks to lifecycle events and coordinates their execution.
- `src/tracker/mod.rs`: Defines the git-tracking namespace.
- `src/tracker/git.rs`: Creates git snapshots and computes file diffs for each phase.
- `src/audit/mod.rs`: Defines the audit data model used to capture per-phase and per-iteration history.
- `src/audit/logger.rs`: Persists audit runs and exports them for inspection.
- `src/compaction/mod.rs`: Defines the context-compaction namespace.
- `src/compaction/config.rs`: Holds the thresholds and tuning knobs for context compaction.
- `src/compaction/tracker.rs`: Tracks compaction sessions, summaries, and related state.
- `src/compaction/summary.rs`: Builds the summary content injected back into the prompt after compaction.
- `src/compaction/manager.rs`: Decides when to compact and coordinates the summary workflow.
- `src/ui/mod.rs`: Defines the terminal UI namespace for sequential and DAG output.
- `src/ui/icons.rs`: Holds the shared icon constants used by the terminal UI.
- `src/ui/progress.rs`: Renders the sequential orchestrator progress bars and status lines.
- `src/ui/dag_progress.rs`: Renders the parallel DAG and swarm execution progress views.
- `src/metrics/mod.rs`: Writes run, phase, iteration, review, and compaction metrics to the Factory database.
- `src/metrics/queries.rs`: Defines the read models used when querying metrics summaries.
- `src/metrics/events.rs`: Is currently a reserved placeholder for a future event-driven metrics pipeline.

Important handoff: `src/dag/executor.rs` reuses the plain per-iteration runner path from `src/orchestrator/runner.rs`; it does not route through the council-specific execution entrypoint.

## 4. Follow Parallel Execution, Reviews, Council, And Swarm

- `src/dag/mod.rs`: Defines the DAG execution namespace and its public scheduler and executor types.
- `src/dag/builder.rs`: Builds a dependency graph from phases and their declared prerequisites.
- `src/dag/scheduler.rs`: Tracks phase readiness and computes waves of executable work.
- `src/dag/state.rs`: Holds DAG execution summaries and per-phase result state.
- `src/dag/executor.rs`: Runs multiple ready phases in parallel and feeds each one through the normal phase loop.
- `src/review/mod.rs`: Defines the review namespace and re-exports the specialist, findings, dispatcher, and arbiter layers.
- `src/review/specialists.rs`: Defines the review specialist catalog and their focus areas.
- `src/review/findings.rs`: Defines review reports, findings, severities, and aggregated verdict data.
- `src/review/prompt_loader.rs`: Loads the specialist prompt templates and prompt modes.
- `src/review/dispatcher.rs`: Dispatches one or more specialists against a phase and collects their reports.
- `src/review/arbiter.rs`: Resolves failed or mixed review outcomes into proceed, fix, or fail decisions.
- `src/council/mod.rs`: Defines the council-execution namespace and re-exports its moving parts.
- `src/council/config.rs`: Defines council and worker-level configuration.
- `src/council/types.rs`: Holds the shared types used during council execution and synthesis.
- `src/council/prompts.rs`: Builds the prompts for chairman, worker, and reviewer roles.
- `src/council/worker.rs`: Defines worker abstractions and the Claude-backed worker implementation.
- `src/council/reviewer.rs`: Runs peer-review rounds over worker output.
- `src/council/chairman.rs`: Synthesizes worker outputs into a final decision or patch set.
- `src/council/merge.rs`: Applies patch sets and manages merge/conflict handling across worktrees.
- `src/council/engine.rs`: Coordinates the full council workflow across workers, review, and synthesis.
- `src/decomposition/mod.rs`: Defines the dynamic decomposition namespace.
- `src/decomposition/config.rs`: Defines thresholds and limits for when decomposition should trigger.
- `src/decomposition/detector.rs`: Decides whether a phase should split into smaller tasks.
- `src/decomposition/parser.rs`: Parses decomposition requests and validates decomposition output.
- `src/decomposition/types.rs`: Holds the types used to describe decomposed tasks and results.
- `src/decomposition/executor.rs`: Executes decomposed work and rolls results back into the parent phase.
- `src/subphase/mod.rs`: Defines the spawned sub-phase namespace.
- `src/subphase/manager.rs`: Tracks the lifecycle and registration of dynamic sub-phases.
- `src/subphase/executor.rs`: Executes child phases that were spawned at runtime.
- `src/swarm/mod.rs`: Defines the Claude swarm namespace.
- `src/swarm/context.rs`: Defines swarm tasks, review config, and execution context objects.
- `src/swarm/prompts.rs`: Builds the orchestration prompts used by swarm execution.
- `src/swarm/callback.rs`: Defines callback messages and server-side callback handling for swarm runs.
- `src/swarm/executor.rs`: Runs a standalone Claude swarm session and resolves it from callback and stdout events.
- `src/autoresearch/mod.rs`: Defines the autoresearch library namespace.
- `src/autoresearch/benchmarks.rs`: Loads and models the benchmark corpus used for specialist evaluation.

## 5. Follow The Factory Backend

The Factory path is a second top-level product surface: the server mounts REST routes, a `/ws` socket, and the embedded SPA, and `POST /api/issues/:id/run` hands control into the pipeline runner.

- `src/factory/mod.rs`: Defines the Factory namespace and gathers its backend modules.
- `src/factory/models.rs`: Holds the typed data model shared across Factory DB, API, and pipeline code.
- `src/factory/embedded.rs`: Embeds the built React UI so the backend can serve it as static assets.
- `src/factory/server.rs`: Builds the Axum app, shared state, REST router, WebSocket endpoint, and static file fallback.
- `src/factory/api.rs`: Implements the REST API for projects, issues, runs, agent teams, GitHub, and metrics.
- `src/factory/ws.rs`: Defines WebSocket messages and the helpers that broadcast them.
- `src/factory/github.rs`: Handles GitHub device flow, token validation, and repo or issue fetches.
- `src/factory/sandbox.rs`: Configures and manages Docker sandbox support for pipeline execution.
- `src/factory/planner.rs`: Defines the planner abstraction and parses the LLM-produced agent-team plan.
- `src/factory/agent_executor.rs`: Executes individual agent tasks, including worktree setup, output parsing, and merge helpers.
- `src/factory/db/mod.rs`: Owns database connection setup for local SQLite and Turso-backed modes.
- `src/factory/db/migrations.rs`: Applies schema migrations for the Factory database.
- `src/factory/db/projects.rs`: Owns CRUD for Factory projects.
- `src/factory/db/issues.rs`: Owns CRUD, movement, and board ordering for issues.
- `src/factory/db/pipeline.rs`: Owns run creation, status updates, and run recovery.
- `src/factory/db/agents.rs`: Owns agent-team, agent-task, and verification persistence.
- `src/factory/db/settings.rs`: Owns persisted settings, including GitHub auth data.
- `src/factory/pipeline/mod.rs`: Coordinates the full pipeline run, chooses execution strategy, and broadcasts lifecycle events.
- `src/factory/pipeline/execution.rs`: Launches `forge swarm`, `forge`, or direct Claude subprocesses for a run.
- `src/factory/pipeline/git.rs`: Owns git locks, branch naming, and worktree-safe git helpers.
- `src/factory/pipeline/parsing.rs`: Translates subprocess output into structured events, progress, and file-change records.

## 6. Follow The Factory React UI

The UI mirrors the backend shape: `App` creates the same-origin WebSocket connection, `useMissionControl` keeps the live project and run cache, and run cards render whatever the backend emits over REST and `/ws`.

- `ui/src/main.tsx`: Boots the React application.
- `ui/src/App.tsx`: Composes the Mission Control shell, derives the WebSocket URL, and wires backend actions into the visible UI.
- `ui/src/index.css`: Defines the shared app styling.
- `ui/src/types/index.ts`: Holds the frontend's typed view of REST and WebSocket payloads.
- `ui/src/api/client.ts`: Implements the typed client for `/api/*`.
- `ui/src/contexts/WebSocketContext.tsx`: Maintains a reconnecting WebSocket connection and subscription API.
- `ui/src/hooks/index.ts`: Re-exports the UI hooks from one place.
- `ui/src/hooks/useMissionControl.ts`: Loads and maintains projects, issues, runs, phases, event logs, and run actions.
- `ui/src/hooks/useAgentTeam.ts`: Maintains the per-run agent-team view, task events, merge state, and verification results.
- `ui/src/components/index.ts`: Re-exports the shared UI components.
- `ui/src/components/ProjectSidebar.tsx`: Renders project navigation and project-level controls.
- `ui/src/components/StatusBar.tsx`: Renders aggregate run status and filter state.
- `ui/src/components/ProjectSetup.tsx`: Handles project creation and repository cloning.
- `ui/src/components/NewIssueModal.tsx`: Handles issue creation inside a project.
- `ui/src/components/FloatingActionButton.tsx`: Exposes the primary quick-create action affordance.
- `ui/src/components/CommandAutocomplete.tsx`: Renders command-style input suggestions.
- `ui/src/components/AgentRunCard.tsx`: Renders the primary run card with phase, team, output, and file-change detail.
- `ui/src/components/Agents.tsx`: Renders the detailed agent-team breakdown for a run.
- `ui/src/components/Analytics.tsx`: Renders pipeline and metrics summaries.
- `ui/src/components/EventLog.tsx`: Renders the live chronological event stream.
- `ui/src/components/ConfirmDialog.tsx`: Renders reusable destructive-action confirmation UI.

## 7. Follow The Workspace Support Crates

One of the parallel agent passes on this walkthrough confirmed the intended layering: `forge-common` defines shared runtime concepts, `forge-proto` defines the gRPC schema for those concepts, and `forge-runtime` implements the daemon that uses both.

### 7.1 `crates/forge-common`

- `crates/forge-common/src/lib.rs`: Re-exports the shared runtime-domain surface for other workspace crates.
- `crates/forge-common/src/ids.rs`: Defines strongly typed ID wrappers for runs, tasks, agents, approvals, and milestones.
- `crates/forge-common/src/manifest.rs`: Defines compiled profiles, agent manifests, capability envelopes, and runtime-environment plans.
- `crates/forge-common/src/policy.rs`: Defines policy types for limits, credentials, approvals, network, and cost controls.
- `crates/forge-common/src/run_graph.rs`: Defines the authoritative in-memory run graph, tasks, milestones, approvals, and scheduling state.
- `crates/forge-common/src/runtime.rs`: Defines the backend-agnostic runtime trait and the agent-launch contract.
- `crates/forge-common/src/events.rs`: Defines the durable runtime event log and message-bus payloads.
- `crates/forge-common/src/output_parser.rs`: Defines the shared line-oriented parser for agent output.
- `crates/forge-common/src/facade.rs`: Defines the shared execution facade abstraction that higher layers call into.
- `crates/forge-common/src/direct_execution.rs`: Implements that facade with direct subprocess execution.

### 7.2 `crates/forge-proto`

- `crates/forge-proto/build.rs`: Compiles `proto/runtime.proto` into generated Rust and gRPC types.
- `crates/forge-proto/src/lib.rs`: Exposes the generated `forge.runtime.v1` types and the conversion helpers.
- `crates/forge-proto/src/convert/mod.rs`: Defines the shared conversion traits and conversion error types.
- `crates/forge-proto/src/convert/enums.rs`: Translates domain enums to and from generated proto enums.
- `crates/forge-proto/src/convert/ids.rs`: Translates strongly typed IDs to and from proto string fields.
- `crates/forge-proto/src/convert/manifest.rs`: Translates profiles, budgets, manifests, and related policy defaults.
- `crates/forge-proto/src/convert/run_graph.rs`: Translates milestones, tasks, and run plans across the wire boundary.

### 7.3 `crates/forge-runtime`

- `crates/forge-runtime/src/lib.rs`: Defines the runtime library surface and resolves daemon socket and state paths.
- `crates/forge-runtime/src/main.rs`: Boots the daemon, opens state, selects a backend, runs recovery, and starts serving.
- `crates/forge-runtime/src/event_stream.rs`: Coordinates durable event replay and live-tail event streaming.
- `crates/forge-runtime/src/profile_compiler.rs`: Validates and compiles trusted base profiles plus project overlays.
- `crates/forge-runtime/src/recovery.rs`: Rebuilds the run graph and reconciles durable state on startup.
- `crates/forge-runtime/src/run_orchestrator.rs`: Owns high-level run submission, approval, and run-graph mutation flows.
- `crates/forge-runtime/src/scheduler.rs`: Chooses which task nodes are ready to launch next.
- `crates/forge-runtime/src/task_manager.rs`: Launches, monitors, and cancels agent instances.
- `crates/forge-runtime/src/server.rs`: Implements the daemon's gRPC server surface.
- `crates/forge-runtime/src/shutdown.rs`: Tracks active agents and coordinates graceful shutdown.
- `crates/forge-runtime/src/version.rs`: Defines protocol-version and daemon-capability constants.
- `crates/forge-runtime/src/runtime/mod.rs`: Selects and configures the concrete runtime backend.
- `crates/forge-runtime/src/runtime/host.rs`: Implements the insecure host-process backend.
- `crates/forge-runtime/src/runtime/docker.rs`: Implements the Docker-container backend.
- `crates/forge-runtime/src/runtime/bwrap.rs`: Implements the bubblewrap sandbox backend.
- `crates/forge-runtime/src/runtime/io.rs`: Shares child-process and container-output plumbing across backends.
- `crates/forge-runtime/src/state/mod.rs`: Defines the SQLite-backed runtime state store.
- `crates/forge-runtime/src/state/schema.rs`: Creates and updates the runtime database schema.
- `crates/forge-runtime/src/state/runs.rs`: Implements CRUD for persisted runs.
- `crates/forge-runtime/src/state/tasks.rs`: Implements CRUD for persisted task nodes.
- `crates/forge-runtime/src/state/events.rs`: Implements append-only event-log persistence and replay helpers.
- `crates/forge-runtime/src/state/agent_instances.rs`: Implements CRUD for persisted agent-instance records.

## Appendix: Test-Only Modules

Inline Rust unit tests live inside many of the source files above. The files in this appendix are the standalone test modules.

### UI tests

- `ui/src/test/setup.ts`: Shared frontend test setup.
- `ui/src/test/fixtures.ts`: Shared fixture data for UI tests.
- `ui/src/test/handlers.ts`: Mock network handlers for frontend tests.
- `ui/src/test/ws-mock.ts`: Mock WebSocket helpers for UI tests.
- `ui/src/test/smoke.test.ts`: Minimal smoke test for the UI entrypoint.
- `ui/src/test/App.test.tsx`: Tests the top-level app shell.
- `ui/src/test/ProjectSetup.test.tsx`: Tests project-setup behavior.
- `ui/src/test/ProjectSidebar.test.tsx`: Tests project-sidebar behavior.
- `ui/src/test/NewIssueModal.test.tsx`: Tests issue-creation flows.
- `ui/src/test/FloatingActionButton.test.tsx`: Tests the floating action affordance.
- `ui/src/test/StatusBar.test.tsx`: Tests run-status summaries.
- `ui/src/test/AgentRunCard.test.tsx`: Tests run-card rendering and behavior.
- `ui/src/test/EventLog.test.tsx`: Tests live event-log rendering.
- `ui/src/test/WebSocketProvider.test.tsx`: Tests WebSocket provider behavior.
- `ui/src/test/useMissionControl.test.ts`: Tests the Mission Control state hook.
- `ui/src/test/useAgentTeam.test.ts`: Tests the agent-team state hook.
- `ui/src/test/mission-control-types.test.ts`: Tests Mission Control-specific type assumptions.
- `ui/src/test/types.test.ts`: Tests general frontend type assumptions.

### Runtime daemon tests

- `crates/forge-runtime/tests/approval_rpcs.rs`: Exercises approval-related RPC behavior.
- `crates/forge-runtime/tests/control_rpcs.rs`: Exercises control RPC behavior.
- `crates/forge-runtime/tests/create_child_task.rs`: Exercises child-task creation behavior.
- `crates/forge-runtime/tests/event_streaming.rs`: Exercises runtime event-stream behavior.
- `crates/forge-runtime/tests/grpc_health.rs`: Exercises daemon health-check behavior.
- `crates/forge-runtime/tests/query_rpcs.rs`: Exercises read/query RPC behavior.
- `crates/forge-runtime/tests/submit_run.rs`: Exercises run-submission behavior.
