//! Runtime daemon SQLite schema.

use anyhow::{Context, Result};
use rusqlite::Connection;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id                 TEXT PRIMARY KEY,
    project            TEXT NOT NULL,
    workspace          TEXT NOT NULL,
    plan_json          TEXT NOT NULL,
    plan_hash          TEXT NOT NULL,
    policy_snapshot    TEXT NOT NULL,
    status             TEXT NOT NULL,
    started_at         TEXT NOT NULL,
    finished_at        TEXT,
    total_tokens       INTEGER DEFAULT 0,
    estimated_cost_usd REAL DEFAULT 0,
    last_event_cursor  INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS task_nodes (
    id                     TEXT PRIMARY KEY,
    run_id                 TEXT NOT NULL REFERENCES runs(id),
    parent_task_id         TEXT REFERENCES task_nodes(id),
    milestone_id           TEXT,
    objective              TEXT NOT NULL,
    expected_output        TEXT NOT NULL,
    profile                TEXT NOT NULL,
    budget                 TEXT NOT NULL,
    memory_scope           TEXT NOT NULL,
    depends_on             TEXT NOT NULL,
    approval_state         TEXT NOT NULL,
    requested_capabilities TEXT NOT NULL,
    worktree               TEXT NOT NULL,
    runtime_mode           TEXT NOT NULL,
    status                 TEXT NOT NULL,
    assigned_agent_id      TEXT,
    created_at             TEXT NOT NULL,
    finished_at            TEXT,
    result_summary         TEXT
);

CREATE TABLE IF NOT EXISTS agent_instances (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES task_nodes(id),
    runtime_backend TEXT NOT NULL,
    pid             INTEGER,
    container_id    TEXT,
    status          TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    resource_peak   TEXT
);

CREATE TABLE IF NOT EXISTS event_log (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     TEXT NOT NULL REFERENCES runs(id),
    task_id    TEXT,
    agent_id   TEXT,
    event_type TEXT NOT NULL,
    payload    TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS credential_access (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp         TEXT NOT NULL,
    agent_id          TEXT NOT NULL,
    credential_handle TEXT NOT NULL,
    action            TEXT NOT NULL,
    context           TEXT
);

CREATE TABLE IF NOT EXISTS memory_access (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    agent_id  TEXT NOT NULL,
    scope     TEXT NOT NULL,
    action    TEXT NOT NULL,
    entry_id  TEXT,
    context   TEXT
);

CREATE INDEX IF NOT EXISTS idx_task_nodes_run_id ON task_nodes(run_id);
CREATE INDEX IF NOT EXISTS idx_task_nodes_parent ON task_nodes(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_task_nodes_status ON task_nodes(status);
CREATE INDEX IF NOT EXISTS idx_event_log_run_id ON event_log(run_id);
CREATE INDEX IF NOT EXISTS idx_event_log_seq ON event_log(seq);
CREATE INDEX IF NOT EXISTS idx_agent_instances_task ON agent_instances(task_id);
"#;

/// Create the runtime schema if it does not already exist.
pub fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)
        .context("failed to create runtime schema tables")?;
    ensure_column(
        conn,
        "runs",
        "workspace",
        "ALTER TABLE runs ADD COLUMN workspace TEXT NOT NULL DEFAULT '.'",
    )?;
    ensure_column(
        conn,
        "task_nodes",
        "worktree",
        "ALTER TABLE task_nodes ADD COLUMN worktree TEXT NOT NULL DEFAULT '{\"Shared\":{\"workspace_path\":\".\"}}'",
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, alter_sql: &str) -> Result<()> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .with_context(|| format!("failed to prepare table info query for {table}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("failed to inspect table info for {table}"))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("failed to collect columns for {table}"))?;

    if columns.iter().any(|existing| existing == column) {
        return Ok(());
    }

    conn.execute_batch(alter_sql)
        .with_context(|| format!("failed to add {column} column to {table}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_creates_all_six_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        create_tables(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare(
                "SELECT name
                 FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();

        assert_eq!(
            tables,
            vec![
                "agent_instances",
                "credential_access",
                "event_log",
                "memory_access",
                "runs",
                "task_nodes",
            ]
        );
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        create_tables(&conn).unwrap();
        create_tables(&conn).unwrap();
    }

    #[test]
    fn schema_migrates_workspace_and_worktree_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE runs (
                id                 TEXT PRIMARY KEY,
                project            TEXT NOT NULL,
                plan_json          TEXT NOT NULL,
                plan_hash          TEXT NOT NULL,
                policy_snapshot    TEXT NOT NULL,
                status             TEXT NOT NULL,
                started_at         TEXT NOT NULL,
                finished_at        TEXT,
                total_tokens       INTEGER DEFAULT 0,
                estimated_cost_usd REAL DEFAULT 0,
                last_event_cursor  INTEGER DEFAULT 0
            );

            CREATE TABLE task_nodes (
                id                     TEXT PRIMARY KEY,
                run_id                 TEXT NOT NULL REFERENCES runs(id),
                parent_task_id         TEXT REFERENCES task_nodes(id),
                milestone_id           TEXT,
                objective              TEXT NOT NULL,
                expected_output        TEXT NOT NULL,
                profile                TEXT NOT NULL,
                budget                 TEXT NOT NULL,
                memory_scope           TEXT NOT NULL,
                depends_on             TEXT NOT NULL,
                approval_state         TEXT NOT NULL,
                requested_capabilities TEXT NOT NULL,
                runtime_mode           TEXT NOT NULL,
                status                 TEXT NOT NULL,
                assigned_agent_id      TEXT,
                created_at             TEXT NOT NULL,
                finished_at            TEXT,
                result_summary         TEXT
            );
            "#,
        )
        .unwrap();

        create_tables(&conn).unwrap();

        let run_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(runs)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(run_columns.iter().any(|column| column == "workspace"));

        let task_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(task_nodes)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(task_columns.iter().any(|column| column == "worktree"));
    }
}
