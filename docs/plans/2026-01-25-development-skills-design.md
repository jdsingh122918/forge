# Development Skills Integration Design

**Date:** 2026-01-25
**Status:** Implemented

## Overview

Integrate five new global skills into Forge to improve code quality during phase execution. These skills provide guidance on TDD, DRY principles, secure coding, library documentation lookup, and independent code review.

## Requirements

1. **TDD** — Apply test-driven development when appropriate
2. **DRY** — Prevent code duplication, encourage reuse
3. **Security** — Follow secure coding practices (OWASP awareness)
4. **Context7** — Query up-to-date library documentation before implementation
5. **Agent Review** — Spawn independent review subagent before phase completion

## Design Decisions

### Approach: Global Skills

Chosen over alternatives:
- **Generation prompt modification** — Would require code changes, less flexible
- **Hooks** — More complex, requires parsing specs for library names
- **Hybrid** — Overkill for instructional guidance

Global skills are:
- Configuration-only (no code changes)
- Independently toggleable per phase
- Easy to maintain and evolve

### Skill Granularity: Five Separate Skills

Chosen over consolidated single skill because:
- Each concern is independently toggleable
- Easier to maintain individually
- Clearer mental model

### Context7 Integration: Instructional

Claude already has MCP access to Context7 tools. The skill teaches *when* to call them rather than automating pre-phase lookups.

### Agent Review: In-Phase Subagent

Review happens within the phase budget via Task tool, before `<promise>DONE</promise>` is emitted. This avoids doubling phase count with explicit review phases.

## Implementation

### New Files Created

```
.forge/skills/
├── tdd-workflow/SKILL.md
├── dry-principles/SKILL.md
├── secure-coding/SKILL.md
├── context7-integration/SKILL.md
└── agent-review/SKILL.md

.forge/forge.toml  (new configuration file)
```

### Configuration

```toml
[skills]
global = [
  "rust-conventions",
  "testing-strategy",
  "cli-design",
  "tdd-workflow",
  "dry-principles",
  "secure-coding",
  "context7-integration",
  "agent-review"
]
```

### Execution Flow

1. `forge run` executes a phase
2. `ClaudeRunner` loads all global skills from `forge.toml`
3. Skills are injected into prompt under `## SKILLS AND CONVENTIONS`
4. During implementation, Claude follows skill guidance
5. Before completion, Claude spawns review subagent
6. Only after review passes does Claude emit `<promise>DONE</promise>`

## Skill Summaries

| Skill | Purpose |
|-------|---------|
| `tdd-workflow` | Red-green-refactor cycle, when to apply TDD |
| `dry-principles` | Duplication detection, Rule of Three, extraction patterns |
| `secure-coding` | OWASP Top 10, input validation, secrets management |
| `context7-integration` | When/how to query Context7 MCP for library docs |
| `agent-review` | Spawning review subagent, handling review results |

## Per-Phase Override

Individual phases can specify their own skills subset:

```json
{
  "number": "05",
  "name": "Write README",
  "skills": ["dry-principles"]
}
```

This overrides the global list for that phase only.
