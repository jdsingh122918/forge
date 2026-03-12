-- Add last_event_at column to pipeline_runs for heartbeat / stall detection.
ALTER TABLE pipeline_runs ADD COLUMN last_event_at TEXT;
