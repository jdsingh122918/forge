# Missing Transaction Wrapper in create_pipeline_run

This code is from the factory database layer that manages CI/CD pipeline runs.
`create_pipeline_run` inserts a new pipeline run row and then reads back the
inserted row using `last_insert_rowid()`.

The problem is that `conn.execute()` and `conn.last_insert_rowid()` are not
wrapped in a transaction. With concurrent connections, another insert could
occur between the execute and the rowid read, causing `last_insert_rowid()` to
return the wrong row ID.

The fix is to wrap the insert + last_insert_rowid in an explicit transaction.
