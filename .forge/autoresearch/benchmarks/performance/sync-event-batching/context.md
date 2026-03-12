# Synchronous Event Batching in Async Runtime

This benchmark is mined from forge commit 06cd7fe, which fixed a real performance bug
in the factory agent executor's event batching system.

The agent executor spawns a background tokio task to batch database writes for agent
events (thinking, output, tool use, etc.). The original implementation used `lock_sync()`
— a synchronous mutex lock — inside the async task, which blocks the tokio runtime thread.

Three performance issues exist:

1. **Blocking the async runtime** (critical): `lock_sync()` blocks the OS thread running
   the tokio task. Since tokio uses a thread pool, this reduces available parallelism and
   can cause other async tasks (WebSocket broadcasts, HTTP handlers) to stall.

2. **Lock held during entire flush** (high): The mutex guard is held while iterating over
   up to 50 events and issuing individual INSERT statements. Other tasks that need the DB
   are starved for the duration of the flush.

3. **No time-based flush** (medium): Events only flush when the batch reaches capacity (50).
   During low-throughput periods, events can sit buffered indefinitely, causing stale data
   in the UI and no delivery time guarantees.

The fix (commit 06cd7fe) replaced `lock_sync()` with fully async database access and added
a `tokio::select!` loop with both count-based (25 events) and time-based (2 second) flush
triggers.
