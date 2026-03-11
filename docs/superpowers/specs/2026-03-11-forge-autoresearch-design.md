# forge autoresearch — Autonomous Agent Optimization

## Overview

Adapt autoresearch-mlx's autonomous experiment loop to systematically improve forge's review specialist agents. The loop mutates specialist prompts, runs them against benchmarks, scores with a cross-model judge, and keeps improvements automatically.

**Goal:** Higher-quality review findings — fewer false positives, better recall of real issues, more actionable fix suggestions.

## Architecture

```
forge autoresearch
├── Benchmark Suite (.forge/autoresearch/benchmarks/)
│   ├── security/          # ~5 real code samples from forge history
│   ├── performance/       # ~5 real code samples
│   ├── architecture/      # ~5 real code samples
│   └── simplicity/        # ~5 real code samples
│
├── Tunable Prompts (.forge/autoresearch/prompts/)
│   ├── security-sentinel.md
│   ├── performance-oracle.md
│   ├── architecture-strategist.md
│   └── simplicity-reviewer.md
│
├── Evaluation Harness
│   ├── Run specialist against benchmarks (Claude)
│   ├── Score outputs (GPT 5.4 via Codex CLI)
│   └── Composite score = 0.4*recall + 0.3*precision + 0.3*actionability
│
├── Experiment Loop (sequential, convergence-based)
│   ├── Claude proposes prompt mutation
│   ├── Run all benchmarks for that specialist
│   ├── Score → keep/discard
│   ├── Stop after 5 consecutive non-improvements
│   └── Move to next specialist
│
├── Budget Tracking
│   └── Hard cap at $25, halt automatically
│
└── Integration
    ├── Audit log (forge's existing system)
    ├── Git branch: autoresearch/<tag>
    ├── results.tsv per specialist
    └── Kept prompts become production configs
```

## Evaluation Design

### Benchmark Suite

Each benchmark is a directory containing:

```
.forge/autoresearch/benchmarks/<specialist>/<case-name>/
├── code.rs          # Code sample to review
├── context.md       # What this code does
├── expected.json    # Ground truth
```

**expected.json format:**
```json
{
  "must_find": [
    { "id": "issue-1", "severity": "critical", "description": "...", "location": "line 42-45" }
  ],
  "must_not_flag": [
    { "id": "fp-1", "description": "This pattern is correct because..." }
  ]
}
```

**Source strategy:** Mine forge's git history for real code that had real issues. Supplement with synthetic examples only if needed.

### Scoring

Per benchmark:
- **Recall** (0-1): proportion of `must_find` items detected
- **Precision** (0-1): 1 - (false_positives / total_findings). `must_not_flag` matches are definite FPs. Novel findings classified by GPT 5.4 judge.
- **Actionability** (0-1): GPT 5.4 rates each fix suggestion (1.0 = specific correct fix, 0.5 = vague, 0.0 = wrong/missing)

**Composite score per specialist:**
```
score = 0.4 * avg_recall + 0.3 * avg_precision + 0.3 * avg_actionability
```

Higher is better.

### Finding Matching Specification

A `must_find` item is considered detected if the specialist output contains a finding whose:
- `file` matches the expected `location` file (if specified), AND
- `line` is within ±5 lines of expected `location` line range, AND
- `issue` description has >50% keyword overlap with expected description

A `must_not_flag` item is considered falsely flagged if:
- A finding's `file`/`line` match the `must_not_flag` location (if specified), OR
- A finding's `issue` has >60% keyword overlap with the `must_not_flag` description

Novel findings (not matching any ground truth entry) are classified by the GPT 5.4 judge.

### Cross-Model Judge (GPT 5.4)

Invoked via Codex CLI:
```bash
codex --model gpt-5.4-xhigh --quiet --json -p "$(cat judge_prompt.txt)"
```

Judge classifies novel findings as true/false positive and rates actionability. Cross-model setup eliminates self-evaluation bias (Claude generates, GPT evaluates).

**Fallback:** If Codex CLI errors, retry once. If still failing, score actionability as 0.5 and classify unknowns as true positives.

## Prompt Config Format

```markdown
---
specialist: SecuritySentinel
mode: gating  # informational only — gating is controlled by SpecialistType::default_gating()
---

## Role
You are a security review specialist...

## Focus Areas
- SQL injection
- XSS
...

## Instructions
...

## Output Format
Return JSON with...
```

YAML frontmatter carries config. Markdown body is the tunable prompt content.

**Production integration:** `build_review_prompt()` in `dispatcher.rs` checks for `.forge/autoresearch/prompts/<specialist>.md`. If found, loads from file (overrides hardcoded default). If not, falls back to current behavior. Rollback = delete the file.

## Experiment Loop

```
SETUP:
1. Create branch: autoresearch/<tag>
2. Extract current specialist prompts → .forge/autoresearch/prompts/
3. Run baseline: score each specialist against its benchmarks
4. Log baseline to results.tsv

LOOP (for each specialist, sequentially):
1. Claude reads: current prompt, results history, benchmark scores
2. Claude proposes a prompt mutation (modifies the .md file)
3. git commit -m "experiment: <description>"
4. Run specialist against all 5 benchmarks → collect outputs
5. GPT 5.4 judges each output → precision, recall, actionability
6. Compute composite score
7. Log to results.tsv
8. If improved → keep (advance branch), reset failure counter
9. If equal/worse → discard (git reset --hard), increment failure counter
10. If 5 consecutive failures → move to next specialist
11. If budget >= $25 → halt

TEARDOWN:
- Print summary: best scores per specialist, improvements vs baseline
- Kept prompts already in production config location
```

## CLI Interface

```bash
forge autoresearch --budget 25 --tag mar11
forge autoresearch --resume mar11
forge autoresearch --results mar11
forge autoresearch --budget 10 --specialist security --tag mar11-sec
forge autoresearch --dry-run --specialist security
```

| Flag | Default | Description |
|------|---------|-------------|
| `--budget` | 25 | Max spend in dollars |
| `--tag` | date-based | Branch: `autoresearch/<tag>` |
| `--specialist` | all | Single specialist mode |
| `--max-failures` | 5 | Consecutive non-improvements before moving on |
| `--resume` | — | Resume from existing results.tsv |
| `--dry-run` | false | One experiment, no git commits |

## Budget

- ~$0.50 per experiment (~5 Claude calls + ~5 GPT 5.4 calls)
- $25 cap → ~50 experiments → ~12 per specialist
- Budget tracked per-call with token-based cost estimates
- Hard halt when ceiling reached

## Constraints

- No new external dependencies beyond Codex CLI (already available)
- No modifications to existing review behavior — additive only (file-exists fallback)
- Benchmarks mined from real forge history where possible
- Sequential specialist optimization (no swarm overhead for v1)

## Future Extensions

- Swarm parallelism across specialists
- Factory UI integration for live experiment monitoring
- Expand to planner, council, and pipeline agents
- A/B testing optimized vs baseline prompts on real forge runs
