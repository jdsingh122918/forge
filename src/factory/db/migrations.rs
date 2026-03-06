use anyhow::{Context, Result};
use libsql::Connection;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("migrations/001_initial.sql")),
    (2, include_str!("migrations/002_github_integration.sql")),
    (3, include_str!("migrations/003_agent_teams.sql")),
    (4, include_str!("migrations/004_settings.sql")),
    (5, include_str!("migrations/005_orchestrator_state.sql")),
    (6, include_str!("migrations/006_soft_deletes_and_indexes.sql")),
];

/// Run all pending migrations. Uses PRAGMA user_version to track state.
pub async fn run_migrations(conn: &Connection) -> Result<()> {
    let current_version = get_user_version(conn).await?;

    // Bootstrap: if tables exist but user_version is 0 (pre-migration DB),
    // detect existing schema and set version accordingly.
    if current_version == 0 {
        let bootstrapped = bootstrap_existing_db(conn).await?;
        if bootstrapped > 0 {
            eprintln!("[db] Bootstrapped existing database at migration version {bootstrapped}");
            return run_from_version(conn, bootstrapped).await;
        }
    }

    run_from_version(conn, current_version).await
}

async fn run_from_version(conn: &Connection, current: i64) -> Result<()> {
    for (version, sql) in MIGRATIONS.iter() {
        if *version <= current {
            continue;
        }
        eprintln!("[db] Running migration {version}...");
        conn.execute_batch(sql)
            .await
            .with_context(|| format!("Failed to run migration {version}"))?;
        set_user_version(conn, *version).await?;
    }
    Ok(())
}

async fn get_user_version(conn: &Connection) -> Result<i64> {
    let mut rows = conn
        .query("PRAGMA user_version", ())
        .await
        .context("Failed to query user_version")?;
    match rows.next().await? {
        Some(row) => Ok(row.get::<i64>(0)?),
        None => Ok(0),
    }
}

async fn set_user_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute(&format!("PRAGMA user_version = {version}"), ())
        .await
        .with_context(|| format!("Failed to set user_version to {version}"))?;
    Ok(())
}

/// Detect if this is a pre-migration database (tables exist but user_version=0).
/// Returns the migration version that matches the current schema, or 0 if empty.
async fn bootstrap_existing_db(conn: &Connection) -> Result<i64> {
    // Check if the core 'projects' table exists
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='projects'",
            (),
        )
        .await?;

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
        .await?;
    if rows.next().await?.is_some() {
        set_user_version(conn, 4).await?;
        return Ok(4);
    }

    // Check for agent_teams table (migration 3)
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='agent_teams'",
            (),
        )
        .await?;
    if rows.next().await?.is_some() {
        set_user_version(conn, 3).await?;
        return Ok(3);
    }

    // Check for github_repo column (migration 2)
    let mut rows = conn.query("PRAGMA table_info(projects)", ()).await?;
    while let Some(row) = rows.next().await? {
        let name: String = row.get(1)?;
        if name == "github_repo" {
            set_user_version(conn, 2).await?;
            return Ok(2);
        }
    }

    // Base tables only (migration 1)
    set_user_version(conn, 1).await?;
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
        let version = get_user_version(&conn).await.unwrap();
        assert_eq!(version, 6);
    }

    #[tokio::test]
    async fn test_idempotent_migration() {
        let (_db, conn) = test_db().await;
        run_migrations(&conn).await.unwrap();
        // Running again should be a no-op
        run_migrations(&conn).await.unwrap();
        let version = get_user_version(&conn).await.unwrap();
        assert_eq!(version, 6);
    }

    #[tokio::test]
    async fn test_partial_migration_resumes() {
        let (_db, conn) = test_db().await;
        // Run only first 2 migrations manually
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[1].1).await.unwrap();
        set_user_version(&conn, 2).await.unwrap();

        // Now run_migrations should pick up from 3
        run_migrations(&conn).await.unwrap();
        let version = get_user_version(&conn).await.unwrap();
        assert_eq!(version, 6);
    }
}
