ALTER TABLE projects ADD COLUMN deleted_at TEXT;
ALTER TABLE issues ADD COLUMN deleted_at TEXT;
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_issue_status ON pipeline_runs(issue_id, status);
CREATE INDEX IF NOT EXISTS idx_pipeline_phases_run_status ON pipeline_phases(run_id, status);
