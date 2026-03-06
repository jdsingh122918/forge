# CLI Autocomplete for StatusBar Command Input

**Date**: 2026-03-06
**Status**: Approved

## Problem

The StatusBar command input (`forge>` prompt) has no autocomplete. Users must remember all available commands and options.

## Solution

Add autocomplete with dropdown suggestions + inline ghost text to the command input.

### Backend: `GET /api/cli-help`

- New endpoint in `src/factory/api.rs`
- Runs `forge --help` (via `FORGE_CMD` env var), parses output into structured JSON
- Returns: `{ commands: [{ name, description }], options: [{ flag, description }] }`
- Response cached in memory (CLI help is static at runtime)

### Frontend: `CommandAutocomplete` component

- New component: `ui/src/components/CommandAutocomplete.tsx`
- Fetches `/api/cli-help` once on mount, caches in state
- Replaces the raw `<input>` in StatusBar
- **Dropdown**: Filtered suggestions below input, keyboard navigable (Up/Down/Enter/Escape)
- **Ghost text**: Top match shown as faded text after cursor position, Tab to accept
- **Styling**: Dark terminal aesthetic using existing `var(--color-*)` tokens

### Files Changed

| File | Change |
|------|--------|
| `src/factory/api.rs` | Add `GET /api/cli-help` endpoint + handler |
| `ui/src/api/client.ts` | Add `cliHelp()` API method |
| `ui/src/components/CommandAutocomplete.tsx` | New autocomplete component |
| `ui/src/components/StatusBar.tsx` | Swap `<input>` for `<CommandAutocomplete>` |

### Autocomplete Data

**Commands**: init, interview, generate, run, phase, list, status, reset, audit, learn, patterns, config, skills, compact, implement, factory, swarm

**Global options**: --verbose, --yes, --auto-approve-threshold, --project-dir, --spec-file, --context-limit, --autonomous

### Behavior

- Dropdown opens on focus or when typing
- Filters as user types (fuzzy or prefix match)
- Arrow Up/Down navigates dropdown, Enter selects
- Escape closes dropdown
- Ghost text shows top match as faded text, Tab accepts
- Selected item populates the input field (cosmetic for now, execution wired later)
