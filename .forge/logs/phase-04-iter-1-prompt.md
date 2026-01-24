You are implementing a project per the following spec.

## SPECIFICATION
# Forge Evolution: The Master's Architecture

## Vision

Evolve Forge from a phase orchestrator into a **disciplined agentic harness** that embodies the master's qualities: calm gating, thoughtful planning, optimized resource usage, and efficient execution. Incorporate proven patterns from Claude Code's architecture while preserving Forge's core strengths.

## Current State

Forge currently provides:
- Phase-based execution with iteration budgets
- Approval gates between phases
- Promise-based completion detection
- Git snapshots and rollback capability
- Comprehensive audit logging
- Pattern learning from completed projects

## Desired Evolution

### 1. Hook System (Event-Driven Extensibility)

Add lifecycle hooks at key decision points, mirroring Claude Code's event model:

**Hook Events:**
- `PrePhase` - Before phase execution (can block, modify prompt, inject context)
- `PostPhase` - After phase completion (can trigger follow-up actions)
- `PreIteration` - Before each Claude invocation
- `PostIteration` - After each Claude response (can analyze, decide to continue)
- `OnFailure` - When phase exceeds budget without promise
- `OnApproval` - When gate is presented (can auto-decide based on context)

**Hook Types:**
- Command hooks (bash scripts receive JSON, exit codes control flow)
- Prompt hooks (small LLM evaluates condition, returns decision)

**Configuration:**
```toml
# .forge/hooks.toml
[[hooks]]
event = "PrePhase"
match = "phase.name contains 'database'"
command = "./scripts/ensure-db-running.sh"

[[hooks]]
event = "PostIteration"
type = "prompt"
prompt = "Did Claude make meaningful progress? Return {continue: bool, reason: str}"
```

### 2. Skills/Templates System

Reusable prompt fragments loaded on-demand, reducing repetition:

**Skill Structure:**
```
.forge/skills/
├── rust-conventions/
│   └── SKILL.md      # Rust-specific guidance
├── testing-strategy/
│   └── SKILL.md      # How to approach tests
└── api-design/
    └── SKILL.md      # REST/GraphQL patterns
```

**Phase Integration:**
```json
{
  "number": "03",
  "name": "API implementation",
  "skills": ["rust-conventions", "api-design"],
  "promise": "API COMPLETE"
}
```

Skills inject their content into phase prompts only when referenced.

### 3. Context Compaction

Prevent context overflow during long-running phases:

**Strategy:**
- Track cumulative context size across iterations
- At threshold (configurable), summarize prior iterations before next prompt
- Preserve: current phase goal, recent code changes, error context
- Discard: verbose intermediate outputs, superseded attempts

**Implementation:**
- Add `--context-limit` flag (default: 80% of model window)
- Add `forge compact` command for manual intervention
- Store compaction summaries in audit trail

### 4. Sub-Phase Delegation

Allow phases to spawn child phases for complex work:

**Use Case:** Phase discovers scope is larger than expected

**Mechanism:**
```
Phase 05: Implement authentication
  └─ Sub-phase 05.1: Set up OAuth provider
  └─ Sub-phase 05.2: Implement JWT handling
  └─ Sub-phase 05.3: Add session management
```

**Rules:**
- Sub-phases inherit parent's git snapshot as baseline
- Sub-phases have independent budgets (from parent's remaining)
- Parent completes only when all sub-phases complete
- Audit trail maintains hierarchy

### 5. Dynamic Permission Modes

Vary oversight based on phase characteristics:

**Modes:**
- `strict` - Require approval for every iteration (sensitive phases)
- `standard` - Approve phase start, auto-continue iterations (default)
- `autonomous` - Auto-approve if within budget and making progress
- `readonly` - Planning/research phases, no file modifications

**Phase Configuration:**
```json
{
  "number": "07",
  "name": "Database migration",
  "permission_mode": "strict",
  "promise": "MIGRATION COMPLETE"
}
```

### 6. Improved Pattern Learning

Capture more nuanced patterns for better future predictions:

**Capture:**
- Phase type classification (scaffold, implement, test, refactor, fix)
- Typical iteration count by phase type
- Common failure modes and recovery patterns
- File change patterns (which files change together)
- Effective prompt structures per phase type

**Apply:**
- Suggest budgets based on phase type and historical data
- Warn when phase structure differs from successful patterns
- Recommend skill loading based on phase classification

### 7. Progress Signaling Improvements

Beyond binary promise detection:

**Intermediate Signals:**
- `<progress>50%</progress>` - Partial completion markers
- `<blocker>Need clarification on X</blocker>` - Explicit blockers
- `<pivot>Changing approach to Y</pivot>` - Strategy shifts

**Forge Response:**
- Progress updates shown in UI
- Blockers can pause and prompt user
- Pivots logged in audit for pattern analysis

### 8. Configuration Unification

Single source of truth for project configuration:

**`.forge/forge.toml`:**
```toml
[project]
name = "my-project"
claude_cmd = "claude"

[defaults]
budget = 8
auto_approve_threshold = 5
permission_mode = "standard"
context_limit = "80%"

[phases.overrides."database-*"]
permission_mode = "strict"
budget = 12

[hooks]
# Hook definitions

[skills]
global = ["rust-conventions"]
```

## Non-Goals

- **Not replacing Claude Code** - Forge orchestrates specs, Claude Code is the execution environment
- **Not building a general agent framework** - Focused on spec-to-implementation workflow
- **Not auto-generating code** - Forge manages the loop, Claude writes the code

## Success Criteria

1. Hooks can intercept and modify any lifecycle event
2. Skills reduce prompt repetition by 50%+
3. Context compaction prevents failures on long phases
4. Sub-phases handle scope discovery gracefully
5. Permission modes provide appropriate oversight levels
6. Pattern learning improves budget accuracy over time
7. Progress signals give visibility into phase internals
8. Single config file replaces scattered settings

## Implementation Priority

1. **Hook system** - Foundation for extensibility
2. **Skills/templates** - Immediate productivity gain
3. **Configuration unification** - Developer experience
4. **Progress signaling** - Visibility improvement
5. **Permission modes** - Safety enhancement
6. **Context compaction** - Robustness for long phases
7. **Pattern learning improvements** - Long-term optimization
8. **Sub-phase delegation** - Advanced capability

## Technical Constraints

- Maintain backward compatibility with existing `.forge/` structure
- Keep Rust as implementation language
- Preserve current CLI interface (additive changes only)
- Audit trail format must remain JSON-exportable
- Git integration via git2 crate continues


## CRITICAL RULES
1. Follow the spec requirements exactly
2. Check existing code before making changes
3. Run tests/checks to verify your work
4. Only output <promise>SKILLS COMPLETE</promise> when the task is FULLY complete and verified
5. If tests fail, fix them before claiming completion

## TASK
Implement phase 04 - Skills and Templates System

When complete, output:
<promise>SKILLS COMPLETE</promise>