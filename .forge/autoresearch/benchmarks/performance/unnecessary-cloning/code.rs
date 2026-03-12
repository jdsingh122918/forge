use std::collections::HashMap;

/// A large event payload from an agent execution stream.
#[derive(Clone, Debug)]
pub struct AgentEvent {
    pub id: i64,
    pub task_id: i64,
    pub event_type: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub timestamp: String,
}

/// Summary statistics for an event stream.
pub struct EventStats {
    pub total: usize,
    pub by_type: HashMap<String, usize>,
    pub total_content_bytes: usize,
    pub error_count: usize,
}

/// Filter events by task ID, returning only matching events.
///
/// BUG (high): Clones every event before checking the filter condition.
/// Should filter first, then clone (or better, return references).
/// For large event streams, this unnecessarily allocates and copies
/// megabytes of content strings and metadata JSON.
pub fn filter_events_by_task(events: &[AgentEvent], task_id: i64) -> Vec<AgentEvent> {
    let mut result = Vec::new();
    for event in events {
        let cloned = event.clone();
        if cloned.task_id == task_id {
            result.push(cloned);
        }
    }
    result
}

/// Compute statistics over an event stream.
///
/// BUG (high): Takes ownership of the events vector unnecessarily. The caller
/// must clone the entire vector to retain access. Should take &[AgentEvent] instead.
/// Additionally, clones event_type strings when a reference would suffice.
pub fn compute_stats(events: Vec<AgentEvent>) -> EventStats {
    let mut by_type: HashMap<String, usize> = HashMap::new();
    let mut total_content_bytes = 0;
    let mut error_count = 0;

    for event in &events {
        let type_key = event.event_type.clone();
        *by_type.entry(type_key).or_insert(0) += 1;
        total_content_bytes += event.content.len();
        if event.event_type == "error" {
            error_count += 1;
        }
    }

    EventStats {
        total: events.len(),
        by_type,
        total_content_bytes,
        error_count,
    }
}

/// Aggregate events from multiple task streams into a combined timeline.
///
/// BUG (medium): Clones all events from every stream into a single vector, then
/// sorts. For N streams of M events each, this allocates N*M clones when a merge
/// iterator or reference-based approach would avoid copies entirely.
pub fn aggregate_timelines(streams: &[Vec<AgentEvent>]) -> Vec<AgentEvent> {
    let mut combined: Vec<AgentEvent> = Vec::new();
    for stream in streams {
        for event in stream {
            combined.push(event.clone());
        }
    }
    combined.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    combined
}

/// Process events in chunks, converting each chunk to a displayable string.
///
/// BUG (medium): Uses chunks() followed by .to_vec() on each chunk, cloning all
/// events in the chunk just to iterate over them. Should iterate over the chunk
/// slice directly without conversion.
pub fn format_event_chunks(events: &[AgentEvent], chunk_size: usize) -> Vec<String> {
    let mut output = Vec::new();
    for chunk in events.chunks(chunk_size) {
        let owned_chunk: Vec<AgentEvent> = chunk.to_vec();
        let mut section = String::new();
        for event in &owned_chunk {
            section.push_str(&format!(
                "[{}] {}: {}\n",
                event.timestamp, event.event_type, event.content
            ));
        }
        output.push(section);
    }
    output
}

/// Count events by type using references — correct, no unnecessary cloning.
pub fn count_by_type(events: &[AgentEvent]) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for event in events {
        *counts.entry(event.event_type.as_str()).or_insert(0) += 1;
    }
    counts
}

/// Get the latest event timestamp — correct, returns a reference.
pub fn latest_timestamp(events: &[AgentEvent]) -> Option<&str> {
    events.iter().map(|e| e.timestamp.as_str()).max()
}
