# Production unwrap() in Signal Parser

This code parses structured signals from Claude's output text during phase
execution. It extracts sub-phase spawn signals encoded as JSON within XML-like
tags.

After pushing a parsed `SubPhaseSpawnSignal` onto the vec, the code calls
`.last().unwrap()` twice to log the name and budget. While `.last()` cannot
currently return `None` (we just pushed), using `.unwrap()` in production
parsing code is dangerous: if the code is refactored (e.g., to drain or filter
the vec between push and log), it will panic on malformed input rather than
returning an error.

The fix is to use the owned `spawn_signal` value for logging before pushing it.
