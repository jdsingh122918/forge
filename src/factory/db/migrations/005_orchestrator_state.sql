CREATE TABLE IF NOT EXISTS orchestrator_state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_context TEXT NOT NULL,
    phase TEXT NOT NULL,
    sub_phase TEXT,
    iteration INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_orchestrator_state_run ON orchestrator_state(run_context);
CREATE INDEX IF NOT EXISTS idx_orchestrator_state_phase ON orchestrator_state(run_context, phase);
