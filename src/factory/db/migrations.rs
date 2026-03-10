use anyhow::{Context, Result};
use libsql::Connection;
use tracing::{debug, info};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("migrations/001_initial.sql")),
    (2, include_str!("migrations/002_github_integration.sql")),
    (3, include_str!("migrations/003_agent_teams.sql")),
    (4, include_str!("migrations/004_settings.sql")),
    (5, include_str!("migrations/005_orchestrator_state.sql")),
    (
        6,
        include_str!("migrations/006_soft_deletes_and_indexes.sql"),
    ),
    (7, include_str!("migrations/007_metrics.sql")),
];

/// Run all pending migrations.
/// Uses a `_migrations` table to track schema version (works across local SQLite and Turso HTTP).
pub async fn run_migrations(conn: &Connection) -> Result<()> {
    ensure_migrations_table(conn).await?;
    let current_version = get_schema_version(conn).await?;

    // Bootstrap: if _migrations table is empty but other tables exist (pre-migration DB),
    // detect existing schema and set version accordingly.
    if current_version == 0 {
        let bootstrapped = bootstrap_existing_db(conn).await?;
        if bootstrapped > 0 {
            info!("Bootstrapped existing database at migration version {bootstrapped}");
            return run_from_version(conn, bootstrapped).await;
        }
    }

    run_from_version(conn, current_version).await
}

/// Create the _migrations tracking table if it doesn't exist.
async fn ensure_migrations_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _migrations (version INTEGER PRIMARY KEY)",
        (),
    )
    .await
    .context("Failed to create _migrations table")?;
    Ok(())
}

/// Get the current schema version from the _migrations table.
async fn get_schema_version(conn: &Connection) -> Result<i64> {
    let mut rows = conn
        .query("SELECT COALESCE(MAX(version), 0) FROM _migrations", ())
        .await
        .context("Failed to query schema version")?;
    match rows.next().await? {
        Some(row) => Ok(row.get::<i64>(0)?),
        None => Ok(0),
    }
}

/// Record that a migration version has been applied.
async fn set_schema_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO _migrations (version) VALUES (?1)",
        [version],
    )
    .await
    .with_context(|| format!("Failed to record migration version {version}"))?;
    Ok(())
}

async fn run_from_version(conn: &Connection, current: i64) -> Result<()> {
    for (version, sql) in MIGRATIONS.iter() {
        if *version <= current {
            continue;
        }
        debug!("Running migration {version}...");
        conn.execute_batch(sql)
            .await
            .with_context(|| format!("Failed to run migration {version}"))?;
        set_schema_version(conn, *version).await?;
    }
    Ok(())
}

/// Detect if this is a pre-migration database (tables exist but no _migrations rows).
/// Returns the migration version that matches the current schema, or 0 if empty.
///
/// Note: Only detects through migration 4 because migrations 5+ were introduced
/// alongside this versioned migration framework and never existed in the pre-migration schema.
async fn bootstrap_existing_db(conn: &Connection) -> Result<i64> {
    // Check if the core 'projects' table exists
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='projects'",
            (),
        )
        .await
        .context("Failed to check for projects table")?;

    if rows.next().await?.is_none() {
        return Ok(0); // Fresh database
    }

    // Tables exist — determine how far the schema has evolved.
    // Check for settings table (migration 4)
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='settings'",
            (),
        )
        .await
        .context("Failed to check for settings table")?;
    if rows.next().await?.is_some() {
        set_schema_version(conn, 4).await?;
        return Ok(4);
    }

    // Check for agent_teams table (migration 3)
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='agent_teams'",
            (),
        )
        .await
        .context("Failed to check for agent_teams table")?;
    if rows.next().await?.is_some() {
        set_schema_version(conn, 3).await?;
        return Ok(3);
    }

    // Check for github_repo column in projects (migration 2)
    // Use sqlite_master to inspect the CREATE TABLE statement instead of PRAGMA table_info
    // since PRAGMA is not supported over Turso HTTP.
    let mut rows = conn
        .query(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='projects'",
            (),
        )
        .await
        .context("Failed to query CREATE TABLE for projects")?;
    if let Some(row) = rows.next().await? {
        let create_sql: String = row.get(0)?;
        if create_sql.contains("github_repo") {
            set_schema_version(conn, 2).await?;
            return Ok(2);
        }
    }

    // Base tables only (migration 1)
    set_schema_version(conn, 1).await?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsql::Builder;

    async fn test_db() -> (libsql::Database, Connection) {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        (db, conn)
    }

    #[tokio::test]
    async fn test_fresh_database_runs_all_migrations() {
        let (_db, conn) = test_db().await;
        run_migrations(&conn).await.unwrap();
        let version = get_schema_version(&conn).await.unwrap();
        assert_eq!(version, 7);
    }

    #[tokio::test]
    async fn test_idempotent_migration() {
        let (_db, conn) = test_db().await;
        run_migrations(&conn).await.unwrap();
        // Running again should be a no-op
        run_migrations(&conn).await.unwrap();
        let version = get_schema_version(&conn).await.unwrap();
        assert_eq!(version, 7);
    }

    #[tokio::test]
    async fn test_partial_migration_resumes() {
        let (_db, conn) = test_db().await;
        ensure_migrations_table(&conn).await.unwrap();
        // Run only first 2 migrations manually
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[1].1).await.unwrap();
        set_schema_version(&conn, 1).await.unwrap();
        set_schema_version(&conn, 2).await.unwrap();

        // Now run_migrations should pick up from 3
        run_migrations(&conn).await.unwrap();
        let version = get_schema_version(&conn).await.unwrap();
        assert_eq!(version, 7);
    }

    #[tokio::test]
    async fn test_bootstrap_empty_database_detects_version_0() {
        let (_db, conn) = test_db().await;
        ensure_migrations_table(&conn).await.unwrap();
        // Empty database — no tables at all (besides _migrations)
        let version = bootstrap_existing_db(&conn).await.unwrap();
        assert_eq!(version, 0);
    }

    #[tokio::test]
    async fn test_bootstrap_detects_version_1_base_tables() {
        let (_db, conn) = test_db().await;
        ensure_migrations_table(&conn).await.unwrap();
        // Apply only migration 1 (initial tables)
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();

        let version = bootstrap_existing_db(&conn).await.unwrap();
        assert_eq!(version, 1);
        // Verify version was recorded
        let sv = get_schema_version(&conn).await.unwrap();
        assert_eq!(sv, 1);
    }

    #[tokio::test]
    async fn test_bootstrap_detects_version_2_github_columns() {
        let (_db, conn) = test_db().await;
        ensure_migrations_table(&conn).await.unwrap();
        // Apply migrations 1 + 2 (github integration)
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[1].1).await.unwrap();

        let version = bootstrap_existing_db(&conn).await.unwrap();
        assert_eq!(version, 2);
        let sv = get_schema_version(&conn).await.unwrap();
        assert_eq!(sv, 2);
    }

    #[tokio::test]
    async fn test_bootstrap_detects_version_3_agent_teams() {
        let (_db, conn) = test_db().await;
        ensure_migrations_table(&conn).await.unwrap();
        // Apply migrations 1 + 2 + 3 (agent teams)
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[1].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[2].1).await.unwrap();

        let version = bootstrap_existing_db(&conn).await.unwrap();
        assert_eq!(version, 3);
        let sv = get_schema_version(&conn).await.unwrap();
        assert_eq!(sv, 3);
    }

    #[tokio::test]
    async fn test_bootstrap_detects_version_4_settings() {
        let (_db, conn) = test_db().await;
        ensure_migrations_table(&conn).await.unwrap();
        // Apply migrations 1 + 2 + 3 + 4 (settings)
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[1].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[2].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[3].1).await.unwrap();

        let version = bootstrap_existing_db(&conn).await.unwrap();
        assert_eq!(version, 4);
        let sv = get_schema_version(&conn).await.unwrap();
        assert_eq!(sv, 4);
    }

    #[tokio::test]
    async fn test_bootstrap_then_upgrade_applies_remaining_migrations() {
        let (_db, conn) = test_db().await;
        // Apply migrations 1 + 2 manually (simulates pre-migration database)
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[1].1).await.unwrap();

        // Run the full migration system — should bootstrap at version 2,
        // then apply migrations 3-7
        run_migrations(&conn).await.unwrap();

        // Verify final version
        let version = get_schema_version(&conn).await.unwrap();
        assert_eq!(version, 7);

        // Verify tables from migrations 3-6 exist and are usable
        // First, set up FK prerequisites: insert a project, issue, and pipeline_run
        conn.execute(
            "INSERT INTO projects (name, path) VALUES ('test-proj', '/tmp/test')",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO issues (project_id, title) VALUES (1, 'test issue')",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO pipeline_runs (issue_id) VALUES (1)", ())
            .await
            .unwrap();

        // Migration 3: agent_teams table
        conn.execute(
            "INSERT INTO agent_teams (run_id, strategy, isolation, plan_summary) VALUES (1, 'sequential', 'none', 'test')",
            (),
        )
        .await
        .unwrap();

        // Migration 4: settings table
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('test_key', 'test_value')",
            (),
        )
        .await
        .unwrap();

        // Migration 5: orchestrator_state table
        conn.execute(
            "INSERT INTO orchestrator_state (run_context, phase, iteration, status) VALUES ('test-run', 'phase1', 1, 'running')",
            (),
        )
        .await
        .unwrap();

        // Migration 6: soft deletes (deleted_at column + indexes)
        // Verify the column exists by doing an update
        conn.execute(
            "UPDATE issues SET deleted_at = datetime('now') WHERE id = -1",
            (),
        )
        .await
        .unwrap();
    }
}
