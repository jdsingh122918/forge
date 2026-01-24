# Forge Implement Command Design

**Date:** 2026-01-24
**Status:** Draft
**Author:** Brainstormed with Claude

## Overview

Add a new `forge implement <design-doc>` command that takes a design document and implements it end-to-end using TDD-first phase generation. This command wraps existing `generate` and `run` functionality, adding design doc extraction and test-first phase pairing.

## Command Interface

```
forge implement <design-doc> [OPTIONS]

ARGUMENTS:
  <design-doc>    Path to design document (markdown)

OPTIONS:
  --no-tdd        Skip test phase generation
  --start-phase   Start from specific phase (for resuming)
  --dry-run       Generate spec and phases, don't execute

EXAMPLES:
  forge implement docs/plans/github-agent-design.md
  forge implement docs/plans/feature.md --no-tdd
  forge implement docs/plans/design.md --start-phase 03
```

## Key Decisions

| Component | Decision | Rationale |
|-----------|----------|-----------|
| Architecture | Wrapper around generate + run | Reuses battle-tested orchestration |
| TDD integration | Phase pairs | Explicit, auditable, adjustable |
| Design doc parsing | Claude-powered extraction | Handles varied doc structures |
| Spec handling | Transform to focused spec | Reduces context consumption |
| Review | Required before execution | Consistent with Forge philosophy |

## Pipeline

```
forge implement docs/plans/design.md
         │
         ▼
┌─────────────────────────────────────────────┐
│  1. PARSE DESIGN DOC                        │
│     • Read markdown file                    │
│     • Validate file exists and is readable  │
└─────────────────┬───────────────────────────┘
                  ▼
┌─────────────────────────────────────────────┐
│  2. EXTRACT & TRANSFORM                     │
│     • Send to Claude with extraction prompt │
│     • Extract: components, priorities,      │
│       dependencies, code patterns           │
│     • Generate implementation-focused spec  │
│     • Save to .forge/spec.md                │
└─────────────────┬───────────────────────────┘
                  ▼
┌─────────────────────────────────────────────┐
│  3. GENERATE TDD PHASES                     │
│     • Claude analyzes spec                  │
│     • For each feature: test phase + impl   │
│     • Assign budgets based on complexity    │
│     • Save to .forge/phases.json            │
└─────────────────┬───────────────────────────┘
                  ▼
┌─────────────────────────────────────────────┐
│  4. USER REVIEW                             │
│     • Display phases with budgets           │
│     • [a]pprove [e]dit [r]egenerate [q]uit  │
│     • Loop until approved or quit           │
└─────────────────┬───────────────────────────┘
                  ▼
┌─────────────────────────────────────────────┐
│  5. EXECUTE (reuse run_orchestrator)        │
│     • Standard phase execution loop         │
│     • Hooks, signals, compaction all work   │
│     • State persisted for resume            │
└─────────────────────────────────────────────┘
```

## Design Doc Extraction Prompt

```rust
pub const DESIGN_DOC_EXTRACTION_PROMPT: &str = r#"You are transforming a design document into an implementation specification.

Given the design document below, extract and generate:

1. **Implementation Spec** - A focused spec for execution (not the full design rationale)
2. **Phase Plan** - TDD phase pairs for each component

## Output Format

Output a JSON object with this exact structure:

{
  "spec": {
    "title": "Implementation title",
    "source": "path/to/design-doc.md",
    "goal": "One paragraph summary of what to build",
    "components": [
      {
        "name": "Component Name",
        "description": "What it does",
        "dependencies": ["other-component"],
        "complexity": "low|medium|high"
      }
    ],
    "patterns": [
      {
        "name": "Pattern name",
        "code": "Code example from design doc"
      }
    ],
    "acceptance_criteria": [
      "Criterion 1",
      "Criterion 2"
    ]
  },
  "phases": [
    {
      "number": "01",
      "name": "Write X tests",
      "promise": "X TESTS WRITTEN",
      "budget": 4,
      "reasoning": "Why these tests",
      "depends_on": [],
      "phase_type": "test"
    },
    {
      "number": "02",
      "name": "Implement X",
      "promise": "X COMPLETE",
      "budget": 10,
      "reasoning": "Implementation details",
      "depends_on": ["01"],
      "phase_type": "implement"
    }
  ]
}

## Guidelines

**For spec extraction:**
- Goal should be actionable, not aspirational
- Components are things that get built (not concepts)
- Extract actual code patterns from design doc code blocks
- Acceptance criteria should be verifiable

**For phase generation:**
- Pair each implementation with a preceding test phase
- Test phases: budget 3-5, promise "X TESTS WRITTEN"
- Implement phases: budget 8-15 based on complexity
- Order by dependency graph (scaffold first, integrations last)
- Use phase_type to distinguish test vs implement

**Budget heuristics:**
- Test phases: 3-5 iterations
- Simple implementation: 8-10 iterations
- Medium complexity: 10-15 iterations
- Complex (database, auth, integrations): 15-20 iterations

## Design Document

"#;
```

## Generated Spec Format

The extracted JSON transforms into `.forge/spec.md`:

```markdown
# Implementation Spec: {title}

> Generated from: {source}
> Generated at: {timestamp}

## Goal

{goal}

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| Webhook Handler | Receives GitHub events | medium | - |
| Worker Pool | Bounded parallelism | medium | - |
| Investigation Agent | Analyzes issues | high | Worker Pool |
| GitHub Client | PR/comment operations | medium | - |

## Code Patterns

### {pattern.name}
```{lang}
{pattern.code}
```

## Acceptance Criteria

- [ ] {criterion_1}
- [ ] {criterion_2}
- [ ] {criterion_3}

---
*This spec was auto-generated from a design document.
Original design: {source}*
```

## Module Structure

```
src/
├── implement/
│   ├── mod.rs           # Public API: run_implement()
│   ├── extract.rs       # Design doc → ExtractedDesign
│   ├── spec_gen.rs      # ExtractedDesign → spec.md
│   └── types.rs         # ExtractedDesign, Component, Pattern structs
├── generate/
│   └── mod.rs           # Existing (reused for phase generation)
├── orchestrator/
│   └── runner.rs        # Existing (reused for execution)
└── main.rs              # Add Implement command
```

## Type Definitions

```rust
// src/implement/types.rs

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractedDesign {
    pub spec: ExtractedSpec,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractedSpec {
    pub title: String,
    pub source: String,
    pub goal: String,
    pub components: Vec<Component>,
    pub patterns: Vec<CodePattern>,
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub description: String,
    pub dependencies: Vec<String>,
    pub complexity: Complexity,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodePattern {
    pub name: String,
    pub code: String,
}
```

## Main Entry Point

```rust
// src/implement/mod.rs

pub fn run_implement(
    project_dir: &Path,
    design_doc: &Path,
    no_tdd: bool,
    dry_run: bool,
) -> Result<()> {
    // 1. Read and validate design doc
    let design_content = validate_design_doc(design_doc)?;

    // 2. Extract structure via Claude
    println!("Parsing design doc...");
    let extracted = extract_design(project_dir, &design_content, design_doc)?;

    // 3. Filter out test phases if --no-tdd
    let phases = if no_tdd {
        extracted.phases.into_iter()
            .filter(|p| p.phase_type.as_deref() != Some("test"))
            .collect()
    } else {
        extracted.phases
    };

    // 4. Generate spec.md
    println!("Generating implementation spec...");
    let spec_content = generate_spec_markdown(&extracted.spec);
    let forge_dir = get_forge_dir(project_dir);
    std::fs::write(forge_dir.join("spec.md"), &spec_content)?;

    // 5. Save phases.json
    println!("Generating phases...");
    let phases_file = create_phases_file(phases.clone(), &spec_content);
    phases_file.save(&forge_dir.join("phases.json"))?;

    // 6. Display phases
    display_phases(&phases);

    if dry_run {
        println!("\nDry run complete. Review:");
        println!("  Spec: .forge/spec.md");
        println!("  Phases: .forge/phases.json");
        return Ok(());
    }

    // 7. Interactive review
    println!("[a]pprove  [e]dit phase  [r]egenerate  [q]uit");
    // ... reuse review loop from generate ...

    // 8. Execute (reuse run_orchestrator)
    // ... call into existing orchestrator ...

    Ok(())
}

fn validate_design_doc(path: &Path) -> Result<String> {
    if !path.exists() {
        bail!("Design doc not found: {}", path.display());
    }

    if path.extension().map(|e| e != "md").unwrap_or(true) {
        bail!("Design doc must be a markdown file (.md)");
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read: {}", path.display()))?;

    if content.trim().is_empty() {
        bail!("Design doc is empty: {}", path.display());
    }

    if !content.contains('#') {
        eprintln!("Warning: Design doc has no markdown headers");
    }

    Ok(content)
}
```

## CLI Integration

```rust
// In src/main.rs

#[derive(Subcommand)]
pub enum Commands {
    // ... existing commands ...

    /// Implement a design document end-to-end with TDD phases
    Implement {
        /// Path to the design document (markdown)
        design_doc: PathBuf,

        /// Skip TDD test phase generation
        #[arg(long)]
        no_tdd: bool,

        /// Start from a specific phase (for resuming)
        #[arg(long)]
        start_phase: Option<String>,

        /// Generate spec and phases without executing
        #[arg(long)]
        dry_run: bool,
    },
}

// In match block:
Commands::Implement { design_doc, no_tdd, start_phase, dry_run } => {
    if let Some(start) = start_phase {
        // Resume from specific phase
        run_orchestrator(&cli, project_dir, Some(start)).await?;
    } else {
        cmd_implement(&project_dir, &design_doc, *no_tdd, *dry_run)?;
    }
}
```

## Error Handling

| Stage | Error | Response |
|-------|-------|----------|
| Read design doc | File not found | `"Design doc not found: {path}"` |
| Read design doc | Not markdown | `"Design doc must be a .md file"` |
| Read design doc | Empty file | `"Design doc is empty"` |
| Extract | Claude timeout | Retry once, then fail with context |
| Extract | Invalid JSON | `"Failed to parse Claude output: {details}"` |
| Extract | No components | `"No implementable components found in design doc"` |
| Generate spec | Write fails | `"Failed to write spec: {io_error}"` |
| Phase review | User quits | Clean exit, partial files preserved |
| Execution | Phase fails | Standard Forge behavior (hooks, retry) |

## Example Session

```
$ forge implement docs/plans/github-agent-design.md

Parsing design doc...
Generating implementation spec...
Generating phases...

Phase 01: Write webhook handler tests        (budget: 4)  [test]
Phase 02: Implement webhook handler          (budget: 10) [implement]
Phase 03: Write worker pool tests            (budget: 4)  [test]
Phase 04: Implement worker pool              (budget: 12) [implement]
Phase 05: Write investigation agent tests    (budget: 5)  [test]
Phase 06: Implement investigation agent      (budget: 15) [implement]
Phase 07: Write GitHub client tests          (budget: 4)  [test]
Phase 08: Implement GitHub client            (budget: 10) [implement]
Phase 09: Integration tests                  (budget: 8)  [test]
Phase 10: End-to-end integration             (budget: 12) [implement]

[a]pprove  [e]dit phase  [r]egenerate  [q]uit
> a

Starting execution...

Phase 01: Write webhook handler tests
  Iteration 1/4 ━━━━━━━━━━ running...
```

---

## V2: Future Enhancements

### Intelligent Phase Merging

**Current (V1):** Fixed test + implementation pairs.

**V2 Vision:**
- Detect when test and implementation are simple enough to merge
- For trivial components, single phase with embedded TDD
- For complex components, keep separate phases
- User can configure merge threshold

```toml
# forge.toml
[implement]
merge_threshold = "low"  # Merge test+impl for low complexity
separate_threshold = "medium"  # Separate for medium+
```

### Design Doc Diffing

**Current (V1):** Full regeneration on design doc changes.

**V2 Vision:**
- Track which design doc generated which phases
- When design doc changes, show diff
- Offer incremental regeneration (only affected phases)
- Preserve completed phases, regenerate pending ones

```
$ forge implement docs/plans/design.md

Design doc changed since last generation.
Changes detected:
  + New component: Rate Limiter
  ~ Modified: Worker Pool (complexity: medium → high)
  - Removed: Legacy Adapter

Options:
  [i]ncremental - Regenerate affected phases only
  [f]ull - Regenerate all phases
  [q]uit
```

### Acceptance Criteria Verification

**Current (V1):** Acceptance criteria listed in spec, not verified.

**V2 Vision:**
- Final phase auto-generated: "Verify acceptance criteria"
- Claude checks each criterion against implementation
- Reports pass/fail for each
- Blocks completion if criteria not met

```rust
// Auto-generated final phase
Phase {
    number: "99",
    name: "Verify acceptance criteria",
    promise: "ALL CRITERIA VERIFIED",
    budget: 5,
    phase_type: "verify",
}
```

### Pattern Learning from Implementations

**Current (V1):** Patterns learned from manual projects.

**V2 Vision:**
- After successful `forge implement`, learn pattern automatically
- Capture: design doc structure → phase structure mapping
- Future design docs benefit from learned mappings
- "This design doc looks like github-agent, suggesting similar phases"

### Parallel Phase Execution

**Current (V1):** Sequential phase execution.

**V2 Vision:**
- Analyze dependency graph
- Run independent phases in parallel
- E.g., "Write webhook tests" and "Write worker pool tests" can run simultaneously
- Requires careful state management

```
Phase 01: Write webhook tests      ─┬─► Phase 02: Implement webhook
Phase 03: Write worker pool tests  ─┴─► Phase 04: Implement worker pool
                                         │
                                         ▼
                                   Phase 05: Integration
```

### Design Doc Templates

**Current (V1):** Any markdown design doc accepted.

**V2 Vision:**
- Provide templates for common design doc types
- Templates have structured sections that extraction understands better
- `forge implement --template api-service` scaffolds design doc
- Better extraction accuracy with known structure

Templates:
- `api-service` — REST/GraphQL service design
- `cli-tool` — Command-line tool design
- `library` — Reusable library design
- `integration` — Third-party integration design

### Confidence Signals

**Current (V1):** Binary success/failure per phase.

**V2 Vision:**
- Claude reports confidence level: `<confidence>85%</confidence>`
- Low confidence triggers human review
- High confidence allows faster auto-approval
- Confidence tracked in audit for learning

```rust
// New signal type
pub enum Signal {
    Progress(u8),
    Blocker(String),
    Pivot(String),
    Confidence(u8),  // New
}
```

### Multi-Design-Doc Projects

**Current (V1):** Single design doc per implementation.

**V2 Vision:**
- Large projects split across multiple design docs
- `forge implement docs/plans/*.md` processes all
- Cross-doc dependency detection
- Unified phase plan across all docs

---

## Implementation Priority

### V1 (Initial Release)
1. CLI command with arguments
2. Design doc validation
3. Claude extraction prompt
4. Spec generation
5. TDD phase pairing
6. Integration with existing generate/run

### V1.1 (Polish)
- Better error messages
- Resume support (`--start-phase`)
- Dry run mode
- Progress display during extraction

### V2 (Intelligence)
- Design doc diffing
- Acceptance criteria verification
- Confidence signals
- Pattern learning

### V3 (Scale)
- Parallel phase execution
- Multi-design-doc projects
- Design doc templates
- Intelligent phase merging
