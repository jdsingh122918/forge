# Unnecessary Cloning in Event Processing

This is a synthetic benchmark showing unnecessary `.clone()` calls on large data structures
in hot-path event processing code. The `AgentEvent` struct contains heap-allocated fields
(String, Option<serde_json::Value>) that are expensive to clone.

Four patterns of unnecessary cloning appear:

1. **Clone-before-filter**: `filter_events_by_task` clones every event before checking
   the filter predicate, wasting allocations for events that are immediately discarded.

2. **Ownership when borrowing suffices**: `compute_stats` takes `Vec<AgentEvent>` by value
   instead of by reference, forcing callers to clone the entire vector to retain access.
   It also clones `event_type` strings for HashMap keys when `&str` references would work.

3. **Clone in aggregation**: `aggregate_timelines` clones all events from all streams into
   a combined vector when a merge iterator over references would avoid copies entirely.

4. **chunks().to_vec()**: `format_event_chunks` calls `.to_vec()` on chunk slices, cloning
   all events in each chunk just to iterate over them. The chunk slice is directly iterable.

The fix is to use references (`&AgentEvent`), borrows (`&[AgentEvent]`), and iterator
adapters to avoid unnecessary heap allocation in the hot path.
