# Session Context

## User Prompts

### Prompt 1

I am running the ./scripts/run-autoresearch-tasks.sh. where is the logs stored?

### Prompt 2

if the forge execution is interrupted - what is the command to resume it?

### Prompt 3

lets list out the individual steps laid out in the ./scripts/run-autoresearch-tasks.sh

### Prompt 4

what are the forge commands to run for all the waves?

### Prompt 5

lets update the ./scripts/run-autonomous-tasks.sh to account for completed phases

### Prompt 6

lets mark the first three tasks in wave 1 i.e. Wave 1:
  # T01
  forge implement
  docs/superpowers/specs/autoresearch-tasks/T01-prompt-config-and-loader.md
  --autonomous --dry-run
  forge reset --force
  forge run --autonomous --yes

  # T04
  forge implement
  docs/superpowers/specs/autoresearch-tasks/T04-benchmark-types-and-loader.md
  --autonomous --dry-run
  forge reset --force
  forge run --autonomous --yes

  # T07
  forge implement
  docs/superpowers/specs/autoresearch-tasks/T07-judg...

### Prompt 7

forge git:(main) ✗ ./scripts/run-autoresearch-tasks.sh
Building forge...
    Finished `release` profile [optimized] target(s) in 23.92s

══════════════════════════════════════
  Wave 1 (5 tasks)
══════════════════════════════════════

The script exited afterwards

