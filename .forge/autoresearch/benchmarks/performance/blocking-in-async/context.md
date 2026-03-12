# Blocking I/O in Async Functions

This is a synthetic benchmark showing multiple instances of blocking operations inside
async functions. The code represents typical forge utility functions for configuration
loading, file checksumming, and external tool execution.

The fundamental problem is using synchronous std library calls (`std::fs::read_to_string`,
`std::fs::read`, `std::process::Command`, `std::fs::metadata`) inside async functions.
These calls block the tokio runtime thread, preventing other async tasks from making
progress. In a server context (like the factory API), this can cause request timeouts
and reduced throughput.

The fixes are:
1. Replace `std::fs` with `tokio::fs` for file I/O
2. Use `tokio::task::spawn_blocking` for CPU-intensive work (hashing)
3. Replace `std::process::Command` with `tokio::process::Command`
