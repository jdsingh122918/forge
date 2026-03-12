# Monolithic Database Module: db.rs (2,400 lines, 8 sections)

This code represents `src/factory/db.rs` before it was refactored in commit e71c83c.
The original file was 2,400 lines containing all database operations in a single module
with 8 distinct sections:

1. **DbHandle** — Async-safe wrapper with spawn_blocking for SQLite
2. **FactoryDb core + migrations** — Database initialization and schema DDL
3. **Project CRUD** — create, list, get, delete projects
4. **Issue CRUD** — create, list, get, move, delete issues with position management
5. **Pipeline runs** — create, update, get pipeline run records
6. **Agent tasks** — agent team and task management
7. **Settings** — key-value settings storage
8. **Row mapping** — IssueRow/PipelineRunRow conversion to domain types

All ~50 database functions live in a single `impl FactoryDb` block, making the file
difficult to navigate, test in isolation, and extend. Adding a new domain area (e.g.,
audit logs) would require modifying this already-massive file.

The refactoring split it into `db/mod.rs`, `db/projects.rs`, `db/issues.rs`,
`db/pipeline.rs`, `db/agents.rs`, and `db/settings.rs`.
