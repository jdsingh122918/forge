# Forge GitHub Agent Design

**Date:** 2026-01-24
**Status:** Draft
**Author:** Brainstormed with Claude

## Overview

A GitHub plugin/agent that automatically responds to new issues by analyzing them, attempting fixes using Forge, and creating PRs for human approval. When fixes aren't possible, it provides detailed analysis and suggested manual steps.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        GitHub                                    │
│  ┌──────────┐    webhook     ┌─────────────────────────────┐   │
│  │  Issue   │ ─────────────► │       GitHub App            │   │
│  │ Created  │                │  (installed on repo)        │   │
│  └──────────┘                └─────────────────────────────┘   │
└──────────────────────────────────┬──────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────┐
│              Forge Agent Service (Railway/Render)                │
│                                                                  │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐         │
│  │   Webhook   │    │   Worker    │    │   Worker    │  (N)    │
│  │   Handler   │───►│   Pool      │    │   Pool      │         │
│  └─────────────┘    └──────┬──────┘    └─────────────┘         │
│                            │                                    │
│         ┌──────────────────┼──────────────────┐                │
│         ▼                  ▼                  ▼                 │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐         │
│  │   Issue     │    │   Issue     │    │   Issue     │         │
│  │  Worker 1   │    │  Worker 2   │    │  Worker 3   │         │
│  │             │    │             │    │             │         │
│  │ clone→investigate→forge→PR     │    │   ...       │         │
│  └─────────────┘    └─────────────┘    └─────────────┘         │
└─────────────────────────────────────────────────────────────────┘
```

### Components

- **GitHub App** — Receives webhooks, provides auth tokens, posts as bot
- **Webhook Handler** — Validates payload, queues work
- **Worker Pool** — Bounded pool (configurable N) processes issues concurrently
- **Issue Worker** — Clones repo, runs investigation, executes Forge, creates PR/comment

## Key Decisions

| Component | Decision | Rationale |
|-----------|----------|-----------|
| Trigger | GitHub webhook | Real-time, no polling delay |
| Hosting | Container on Railway/Render | Long-running, persistent filesystem |
| Issue scope | Attempt all issues | Bold/ambitious — agent as first responder |
| Failure mode | Comment + suggest steps | Provides value even on failure |
| Codebase | Fresh clone per issue | Complete isolation, simple cleanup |
| Pipeline | Investigate → Forge | Investigation scopes problem first |
| Investigation output | `.forge/spec.md` | Leverages Forge's phase system |
| Concurrency | Bounded parallelism | Controlled resource usage |
| Auth (GitHub) | GitHub App | Built-in webhooks, short-lived tokens |
| Auth (Claude) | `CLAUDE_CODE_OAUTH_TOKEN` | Subscription-based, PaaS-friendly |
| PR format | Code + explanation | Context for reviewers |
| Stack | Rust, axum, octocrab, tokio | Consistent with Forge |

## Issue Worker Pipeline

```
┌─────────────┐
│ 1. SETUP    │  • Clone repo to temp directory
│             │  • Checkout default branch
│             │  • Create feature branch: forge/issue-{number}
└─────┬───────┘
      ▼
┌─────────────┐
│ 2. INVESTIGATE │  • Run Claude with investigation prompt
│                │  • Inputs: issue title, body, comments, repo structure
│                │  • Outputs: analysis + generated .forge/spec.md
│                │  • Decision: fixable (proceed) or unfixable (skip to 5)
└─────┬──────────┘
      ▼
┌─────────────┐
│ 3. EXECUTE  │  • Run `forge run` with generated spec
│             │  • Permission mode: autonomous
│             │  • Capture output and audit trail
└─────┬───────┘
      ▼
┌─────────────┐
│ 4. PUBLISH  │  • If Forge succeeded: push branch, create PR
│             │  • PR links to issue, includes explanation
│             │  • Request review (optional: specific reviewers)
└─────┬───────┘
      ▼
┌─────────────┐
│ 5. REPORT   │  • If failed anywhere: comment on issue
│             │  • Include: what was tried, where it failed
│             │  • Suggest manual steps for human developer
└─────┬───────┘
      ▼
┌─────────────┐
│ 6. CLEANUP  │  • Delete temp directory
│             │  • Log outcome for metrics
└─────────────┘
```

## Investigation Agent

### Input Context

- Issue title and body
- Issue comments (if any)
- Repository file tree (top-level + key directories)
- README.md content
- Recent commit messages (last 10-20)
- Stack traces or error messages from issue body

### Output

A generated `.forge/spec.md`:

```markdown
# Fix: Login fails when email contains plus sign

## Context
Issue #42 reports login failure for emails like "user+tag@example.com".
Root cause: email validation regex in `src/auth/validate.rs:47`
rejects valid RFC 5321 characters.

## Phases

### Phase 01: Fix email validation
- Budget: 5
- Update regex in `src/auth/validate.rs` to accept + character
- Promise: VALIDATION_FIXED

### Phase 02: Add test coverage
- Budget: 3
- Add test cases for plus-sign emails
- Promise: TESTS_PASSING
```

If unfixable, returns structured analysis explaining why.

## GitHub App Configuration

```
Name: Forge Agent
Webhook URL: https://your-service.railway.app/webhook
Webhook Secret: <generated>

Permissions:
  - Issues: Read
  - Pull Requests: Write
  - Contents: Write
  - Metadata: Read

Events:
  - Issues (opened)
```

## Implementation

### Tech Stack

- **Language:** Rust (consistent with Forge)
- **Web framework:** axum
- **Job queue:** tokio bounded channels
- **Git operations:** git2 crate
- **GitHub API:** octocrab crate

### Project Structure

```
forge-github-agent/
├── Cargo.toml
├── Dockerfile
├── src/
│   ├── main.rs              # Service entrypoint
│   ├── config.rs            # Env vars, settings
│   ├── webhook/
│   │   ├── mod.rs
│   │   ├── handler.rs       # POST /webhook endpoint
│   │   └── verify.rs        # Signature verification
│   ├── worker/
│   │   ├── mod.rs
│   │   ├── pool.rs          # Bounded worker pool
│   │   └── issue_worker.rs  # Pipeline implementation
│   ├── investigate/
│   │   ├── mod.rs
│   │   ├── context.rs       # Gather repo context
│   │   ├── prompt.rs        # Investigation prompt template
│   │   └── spec_gen.rs      # Parse output → spec file
│   ├── github/
│   │   ├── mod.rs
│   │   ├── auth.rs          # App auth, installation tokens
│   │   ├── issues.rs        # Fetch issue, post comments
│   │   └── pulls.rs         # Create PRs
│   └── metrics.rs           # Success/failure tracking
└── prompts/
    └── investigate.md       # Investigation prompt template
```

### Code Patterns

**Webhook handler (axum + octocrab):**

```rust
use axum::{Router, routing::post, extract::State, Json};
use octocrab::models::webhook_events::{WebhookEvent, WebhookEventType, WebhookEventPayload};

async fn handle_webhook(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> StatusCode {
    // Verify signature

    // Type-safe event parsing
    let event = WebhookEvent::try_from_header_and_body(
        headers.get("X-GitHub-Event").unwrap().to_str().unwrap(),
        &body
    ).unwrap();

    if let WebhookEventType::Issues = event.kind {
        if let WebhookEventPayload::Issues(issue_event) = event.specific {
            if issue_event.action == IssuesEventAction::Opened {
                // Queue for processing (bounded channel)
                app_state.job_tx.send(issue_event).await.ok();
            }
        }
    }

    StatusCode::ACCEPTED
}

let app = Router::new()
    .route("/webhook", post(handle_webhook))
    .with_state(app_state);
```

**GitHub App authentication:**

```rust
use octocrab::Octocrab;
use jsonwebtoken::EncodingKey;

let key = EncodingKey::from_rsa_pem(private_key.as_bytes())?;
let app_client = Octocrab::builder()
    .app(app_id.into(), key)
    .build()?;

// Get installation-specific client
let (client, _token) = app_client
    .installation_and_token(installation_id)
    .await?;
```

**Bounded worker pool:**

```rust
use tokio::sync::mpsc;

let (tx, rx) = mpsc::channel::<IssueJob>(100);

for _ in 0..config.max_workers {
    let mut rx = rx.clone();
    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            process_issue(job).await;
        }
    });
}
```

**Invoking Claude:**

```rust
let output = Command::new("claude")
    .args(["-p", &prompt, "--allowedTools", "Read,Edit,Bash,Write", "--output-format", "json"])
    .current_dir(&work_dir)
    .output()
    .await?;
```

## Deployment

### Dockerfile

```dockerfile
FROM rust:1.75-slim as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install Claude CLI
RUN curl -fsSL https://cli.claude.ai/install.sh | sh

COPY --from=builder /app/target/release/forge-github-agent /usr/local/bin/
COPY --from=builder /app/target/release/forge /usr/local/bin/

EXPOSE 8080
CMD ["forge-github-agent"]
```

### Environment Variables

```bash
# GitHub App
GITHUB_APP_ID=123456
GITHUB_APP_PRIVATE_KEY="-----BEGIN RSA PRIVATE KEY-----\n..."
GITHUB_WEBHOOK_SECRET=your-webhook-secret

# Claude (subscription-based)
CLAUDE_CODE_OAUTH_TOKEN=your-oauth-token

# Service config
PORT=8080
MAX_WORKERS=3
WORK_DIR=/tmp/forge-work
LOG_LEVEL=info
```

### Setup Steps

1. Create GitHub App with required permissions
2. Generate and save private key
3. Run `claude login` then `claude setup-token` locally
4. Deploy to Railway/Render with env vars
5. Install GitHub App on target repositories

## Error Handling

| Stage | Error | Response |
|-------|-------|----------|
| Webhook | Invalid signature | 401, log attempt |
| Webhook | Malformed payload | 400, log error |
| Clone | Git clone fails | Comment: "Couldn't access repository" |
| Investigate | Claude timeout | Comment: "Investigation failed" |
| Investigate | Deemed unfixable | Comment: analysis + why |
| Forge | Phase fails budget | Comment: attempt details + stuck point |
| Forge | Tests fail | Comment: partial progress + failures |
| Push | Branch push fails | Retry once, then comment failure |
| PR | PR creation fails | Comment with branch name |

### Comment Templates

**Success:**
```markdown
## 🤖 Forge Agent

I've analyzed this issue and created a fix: #87

**What I found:**
- Root cause: Email validation regex rejects `+` character
- Files changed: `src/auth/validate.rs`, `tests/auth_test.rs`

**Testing notes:**
- Added test cases for plus-sign emails
- All existing tests pass

Please review and merge if the fix looks correct.
```

**Failure:**
```markdown
## 🤖 Forge Agent

I attempted to fix this issue but wasn't able to complete it automatically.

**What I tried:**
- Identified relevant files: `src/api/handler.rs`
- Attempted fix in Phase 1, but tests failed after changes

**Where I got stuck:**
- The fix requires changes to the database schema
- This needs human decision on migration strategy

**Suggested next steps:**
1. Review the schema in `migrations/002_users.sql`
2. Decide on backward compatibility approach
3. Update handler after migration is in place
```

## Observability

### Logging

```rust
use tracing::{info, error, instrument};

#[instrument(skip(app_state), fields(issue = %issue.number, repo = %repo.full_name))]
async fn process_issue(issue: Issue, repo: Repository, app_state: AppState) {
    info!("Starting issue processing");
    // ...
}
```

### Metrics

- `issues_received` — Total webhooks received
- `issues_processed` — Issues that entered pipeline
- `issues_succeeded` — PRs created
- `issues_failed` — Comments posted on failure
- `investigation_duration` — Time spent in investigation
- `forge_duration` — Time spent in Forge execution
- `outcomes` — Counts by outcome type

### Endpoints

- `GET /health` — Liveness check for platform
- `GET /status` — Worker pool status, queue depth, recent issues
- `GET /metrics` — Prometheus format (optional)

---

## V2: Future Enhancements

### Progressive Budget Relaxation

**Current (V1):** Fixed budgets per phase (e.g., 5 iterations).

**V2 Vision:**
- Track success rates per repository and issue type
- Automatically increase budgets for repos with high success rates
- Decrease budgets for repos that consistently fail (save compute)
- Learn optimal budget allocation from historical data

**Implementation ideas:**
```rust
struct BudgetStrategy {
    base_budget: u32,
    repo_multipliers: HashMap<RepoId, f32>,  // learned over time
    issue_type_multipliers: HashMap<IssueType, f32>,
}

fn calculate_budget(repo: &Repo, issue: &Issue) -> u32 {
    let base = strategy.base_budget;
    let repo_mult = strategy.repo_multipliers.get(&repo.id).unwrap_or(&1.0);
    let type_mult = strategy.issue_type_multipliers.get(&issue.classified_type).unwrap_or(&1.0);
    (base as f32 * repo_mult * type_mult).ceil() as u32
}
```

### Self-Healing Capabilities

**Current (V1):** Single attempt, comment on failure.

**V2 Vision:**
- If Forge fails, analyze the failure pattern
- Retry with adjusted strategy:
  - Different phase structure
  - More context in prompts
  - Alternative approaches
- Learn from failure modes to prevent similar failures

**Implementation ideas:**
```rust
enum RetryStrategy {
    IncreaseBudget,           // Same approach, more iterations
    AddContext(Vec<FilePath>), // Include more files in prompt
    SimplifyPhases,           // Fewer, broader phases
    AlternativeApproach,      // Let Claude try different strategy
}

async fn attempt_with_retry(issue: &Issue, max_attempts: u32) -> Result<PR, FailureReport> {
    let mut strategy = RetryStrategy::default();
    for attempt in 0..max_attempts {
        match execute_forge(&issue, &strategy).await {
            Ok(pr) => return Ok(pr),
            Err(failure) => {
                strategy = analyze_failure_and_adjust(&failure);
                log_retry_attempt(attempt, &failure, &strategy);
            }
        }
    }
    Err(aggregate_failure_report())
}
```

### Feedback Loop & Learning

**Current (V1):** No learning from outcomes.

**V2 Vision:**
- Track which PRs get merged vs closed
- Feed merge/close outcomes back into investigation tuning
- Build repo-specific "knowledge" about what works
- Learn from manual fixes — when humans fix issues Forge failed on, analyze the diff

**Data to collect:**
```rust
struct OutcomeRecord {
    issue_id: IssueId,
    repo_id: RepoId,

    // What we tried
    investigation_analysis: String,
    generated_spec: Spec,
    forge_phases_completed: u32,

    // What happened
    outcome: Outcome,  // PrMerged, PrClosed, PrSuperseded, FailedToFix

    // If human fixed it
    human_fix_diff: Option<Diff>,
    time_to_human_fix: Option<Duration>,
}

// Periodically analyze outcomes
async fn learn_from_outcomes() {
    let recent = db.get_outcomes(last_30_days);

    // Which repos have high merge rates?
    let repo_success_rates = calculate_repo_success_rates(&recent);
    update_budget_multipliers(repo_success_rates);

    // What patterns in issues correlate with failure?
    let failure_patterns = analyze_failure_patterns(&recent);
    update_investigation_prompt(failure_patterns);

    // What did humans do differently on our failures?
    let human_fixes = recent.filter(|r| r.human_fix_diff.is_some());
    extract_learnings(human_fixes);
}
```

### Additional V2 Features

**Label-based configuration:**
- `forge:skip` — Don't attempt this issue
- `forge:high-priority` — Process immediately, higher budget
- `forge:simple` — Use minimal investigation, direct fix

**Multi-repo installations:**
- Single agent instance serving multiple repos
- Per-repo configuration overrides
- Shared learning across similar repos

**PR follow-up:**
- Monitor PR review comments
- Attempt to address reviewer feedback automatically
- Learn from review patterns to improve initial PRs

**Integration with Forge hooks:**
- Use Forge's existing hook system for custom logic
- Pre-investigation hooks for repo-specific setup
- Post-PR hooks for notification/integration

---

## Implementation Priority

### V1 (Initial Release)
1. GitHub App + webhook handler
2. Worker pool with bounded concurrency
3. Investigation agent + spec generation
4. Forge integration (autonomous mode)
5. PR creation + failure comments
6. Basic observability (logs, health endpoint)

### V1.1 (Polish)
- Retry logic for transient failures
- Better error messages in comments
- Metrics endpoint
- Rate limiting

### V2 (Learning)
- Outcome tracking database
- Progressive budget adjustment
- Self-healing retry strategies
- Feedback loop analysis

### V3 (Advanced)
- Multi-repo optimization
- PR follow-up automation
- Cross-repo learning
- Custom per-repo configurations
