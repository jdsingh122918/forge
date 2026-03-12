# Tight Coupling: Direct rusqlite Dependency Throughout Database Layer

This code represents `src/factory/db.rs` before commit 96c6b63 replaced rusqlite with
libsql. Every database function directly uses `rusqlite::Connection`, `rusqlite::params!`,
and `rusqlite::Row` — there is no trait abstraction, repository interface, or driver
abstraction layer.

The tight coupling means:
- Swapping the database driver (rusqlite -> libsql) requires rewriting all ~50 functions
- Unit testing requires a real SQLite connection (no mock/stub possible)
- The `row.get(N)` positional column access is repeated in every function with no
  shared row-mapping helper, leading to fragile index-based access throughout
- Every function constructs SQL strings inline and uses `params![]` macro directly

The actual migration in commit 96c6b63 required touching every single function in the
file to change `rusqlite::` types to `libsql::` equivalents, demonstrating the real
cost of this coupling.
