use anyhow::{Context, Result};
use libsql::Connection;

pub async fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut rows = conn
        .query("SELECT value FROM settings WHERE key = ?1", [key])
        .await
        .context("Failed to query setting")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get::<String>(0)?)),
        None => Ok(None),
    }
}

pub async fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
        (key, value),
    )
    .await
    .context("Failed to upsert setting")?;
    Ok(())
}

pub async fn delete_setting(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])
        .await
        .context("Failed to delete setting")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;

    #[tokio::test]
    async fn test_set_and_get_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        set_setting(&conn, "test_key", "test_value").await.unwrap();
        let val = get_setting(&conn, "test_key").await.unwrap();
        assert_eq!(val, Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_get_missing_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        let val = get_setting(&conn, "nonexistent").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_upsert_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        set_setting(&conn, "key", "v1").await.unwrap();
        set_setting(&conn, "key", "v2").await.unwrap();
        let val = get_setting(&conn, "key").await.unwrap();
        assert_eq!(val, Some("v2".to_string()));
    }

    #[tokio::test]
    async fn test_delete_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        set_setting(&conn, "key", "value").await.unwrap();
        delete_setting(&conn, "key").await.unwrap();
        let val = get_setting(&conn, "key").await.unwrap();
        assert_eq!(val, None);
    }
}
