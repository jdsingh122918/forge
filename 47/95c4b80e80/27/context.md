# Session Context

## User Prompts

### Prompt 1

You are implementing a project per the following spec.

## SPECIFICATION
# Implementation Spec: Wire PromptLoader into Review Dispatcher

> Generated from: docs/superpowers/specs/autoresearch-tasks/T03-wire-into-dispatcher.md
> Generated at: 2026-03-12T01:38:29.680606+00:00

## Goal

Modify build_review_prompt() in dispatcher.rs to accept an optional forge_dir parameter. When provided and a valid prompt file exists, use the file-based prompt body with dynamic context injection. When no file e...

