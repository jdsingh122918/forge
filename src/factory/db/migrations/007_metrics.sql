-- Metrics tables for observability analytics

CREATE TABLE IF NOT EXISTS metrics_runs (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE,
    issue_id INTEGER,
    success INTEGER NOT NULL DEFAULT 0,
    phases_total INTEGER,
    phases_passed INTEGER,
    duration_secs REAL,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS metrics_phases (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES metrics_runs(run_id),
    phase_number INTEGER NOT NULL,
    phase_name TEXT NOT NULL,
    budget INTEGER NOT NULL,
    iterations_used INTEGER,
    outcome TEXT,
    duration_secs REAL,
    files_added INTEGER DEFAULT 0,
    files_modified INTEGER DEFAULT 0,
    files_deleted INTEGER DEFAULT 0,
    lines_added INTEGER DEFAULT 0,
    lines_removed INTEGER DEFAULT 0,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    UNIQUE(run_id, phase_number)
);

CREATE TABLE IF NOT EXISTS metrics_iterations (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL,
    phase_number INTEGER NOT NULL,
    iteration INTEGER NOT NULL,
    duration_secs REAL,
    prompt_chars INTEGER,
    output_chars INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    progress_percent INTEGER,
    blocker_count INTEGER DEFAULT 0,
    pivot_count INTEGER DEFAULT 0,
    promise_found INTEGER DEFAULT 0,
    FOREIGN KEY (run_id, phase_number) REFERENCES metrics_phases(run_id, phase_number)
);

CREATE TABLE IF NOT EXISTS metrics_reviews (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL,
    phase_number INTEGER NOT NULL,
    specialist_type TEXT NOT NULL,
    verdict TEXT NOT NULL,
    findings_count INTEGER DEFAULT 0,
    critical_count INTEGER DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id, phase_number) REFERENCES metrics_phases(run_id, phase_number)
);

CREATE TABLE IF NOT EXISTS metrics_compactions (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL,
    phase_number INTEGER NOT NULL,
    iterations_compacted INTEGER,
    original_chars INTEGER,
    summary_chars INTEGER,
    compression_ratio REAL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (run_id, phase_number) REFERENCES metrics_phases(run_id, phase_number)
);

CREATE INDEX IF NOT EXISTS idx_metrics_runs_started ON metrics_runs(started_at);
CREATE INDEX IF NOT EXISTS idx_metrics_runs_issue ON metrics_runs(issue_id);
CREATE INDEX IF NOT EXISTS idx_metrics_phases_outcome ON metrics_phases(outcome);
CREATE INDEX IF NOT EXISTS idx_metrics_iterations_tokens ON metrics_iterations(input_tokens, output_tokens);
