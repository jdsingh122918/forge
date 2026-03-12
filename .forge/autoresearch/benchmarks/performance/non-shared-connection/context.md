# Non-Shared Database Connection in DbHandle

This benchmark is mined from forge commit 54b9a63, which fixed a real performance and
correctness bug in the factory database handle.

The `DbHandle` struct wraps a `libsql::Database` and provides convenience methods for
projects, issues, and other entities. Each convenience method calls `conn()` to get a
database connection. The bug is that `conn()` creates a **new connection** on every call
via `self.db.connect()`.

Three issues stem from this design:

1. **In-memory database isolation** (critical): For `:memory:` SQLite databases (used in
   testing), each `connect()` call creates a completely separate in-memory database. Data
   written through one connection is invisible to another, causing test failures and data
   loss.

2. **Connection overhead** (high): For file-based databases, each `connect()` incurs the
   cost of opening file handles, negotiating WAL mode, and setting pragmas. These should
   be amortized across many operations.

3. **No connection reuse** (medium): Connections are created per-call and discarded
   immediately, with no pooling or caching. In high-throughput scenarios this wastes
   file descriptors and memory.

The fix (commit 54b9a63) changed `DbHandle` to store a shared `Connection` (created once
at construction) and return `&Connection` from `conn()` instead of creating a new one.
