use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

/// Database handle wrapping a connection pool.
#[derive(Clone)]
pub struct DbWriter {
    inner: Arc<Mutex<DbInner>>,
}

struct DbInner {
    conn: rusqlite::Connection,
}

impl DbWriter {
    /// Acquire a synchronous lock on the database — blocks the current thread.
    /// BUG: Calling lock_sync() inside a tokio task blocks the async runtime thread,
    /// preventing other tasks from making progress while the lock is held.
    pub fn lock_sync(&self) -> Result<std::sync::MutexGuard<'_, DbInner>, String> {
        match self.inner.blocking_lock_owned() {
            guard => Ok(unsafe { std::mem::transmute(guard) }),
        }
    }
}

impl DbInner {
    fn create_event(&self, task_id: i64, event_type: &str, content: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO agent_events (task_id, event_type, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![task_id, event_type, content],
        )?;
        Ok(())
    }
}

/// Spawns an event writer task that batches agent events for DB writes.
///
/// BUG 1 (critical): Uses lock_sync() inside a tokio::spawn task, blocking the async
/// runtime thread. Should use async-aware locking or restructure to avoid blocking.
///
/// BUG 2 (high): Lock is held for the entire batch flush loop. If the batch is large
/// (up to 50 events), other tasks needing the DB are starved.
///
/// BUG 3 (medium): No time-based flush — events only flush when the batch reaches 50.
/// Low-throughput periods can leave events buffered indefinitely with no delivery guarantee.
pub fn spawn_event_writer(
    db_writer: DbWriter,
    mut event_rx: mpsc::UnboundedReceiver<(i64, String, String)>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut batch = Vec::new();
        loop {
            // Wait for first event
            match event_rx.recv().await {
                Some(event) => batch.push(event),
                None => break, // Channel closed
            }
            // Drain any additional ready events (up to 50)
            while batch.len() < 50 {
                match event_rx.try_recv() {
                    Ok(event) => batch.push(event),
                    Err(_) => break,
                }
            }
            // Flush batch to DB using lock_sync() — blocks the async runtime thread!
            {
                let db = match db_writer.lock_sync() {
                    Ok(db) => db,
                    Err(e) => {
                        eprintln!("[agent] DB lock poisoned, stopping event writer: {}", e);
                        break;
                    }
                };
                // Lock held for entire flush loop — starves other DB users
                for (task_id, event_type, content) in batch.drain(..) {
                    if let Err(e) = db.create_event(task_id, &event_type, &content) {
                        eprintln!("[agent] Failed to write event: {:#}", e);
                    }
                }
            }
        }
        // Flush remaining events on shutdown
        if !batch.is_empty() {
            let db = match db_writer.lock_sync() {
                Ok(db) => db,
                Err(e) => {
                    eprintln!(
                        "[agent] DB lock poisoned, dropping {} unflushed events: {}",
                        batch.len(),
                        e
                    );
                    return;
                }
            };
            for (task_id, event_type, content) in batch.drain(..) {
                if let Err(e) = db.create_event(task_id, &event_type, &content) {
                    eprintln!("[agent] Failed to flush event for task {}: {:#}", task_id, e);
                }
            }
        }
    })
}

/// Sends events through the channel — safe, no performance issues here.
pub async fn emit_event(
    tx: &mpsc::UnboundedSender<(i64, String, String)>,
    task_id: i64,
    event_type: &str,
    content: &str,
) {
    let _ = tx.send((task_id, event_type.to_string(), content.to_string()));
}

/// Broadcasts a WebSocket message — safe, non-blocking operation.
pub fn broadcast_ws(tx: &tokio::sync::broadcast::Sender<String>, msg: &str) {
    let _ = tx.send(msg.to_string());
}
