//! Factory Kanban UI server command — `forge factory`.

use anyhow::Result;
use tracing::warn;

pub async fn cmd_factory(
    port: u16,
    init: bool,
    db_path: std::path::PathBuf,
    open: bool,
    dev: bool,
) -> Result<()> {
    if init {
        // Just initialize the database (creates + runs migrations)
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        forge::factory::db::DbHandle::new_local(&db_path).await?;
        println!("Factory database initialized at {}", db_path.display());
        return Ok(());
    }

    // Spawn browser open before starting the server (which blocks)
    // Skip in dev mode (no browser inside Docker containers)
    if open && !dev {
        let url = format!("http://localhost:{}", port);
        tokio::spawn(async move {
            // Small delay to let the server start binding
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if let Err(e) = open::that(&url) {
                warn!("Failed to open browser: {}", e);
            }
        });
    }

    let project_path = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    forge::factory::server::start_server(forge::factory::server::ServerConfig {
        port,
        db_path,
        project_path,
        dev_mode: dev,
    })
    .await?;

    Ok(())
}
