CREATE TABLE IF NOT EXISTS agent_teams (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES pipeline_runs(id),
    strategy TEXT NOT NULL,
    isolation TEXT NOT NULL,
    plan_summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS agent_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    team_id INTEGER NOT NULL REFERENCES agent_teams(id),
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    agent_role TEXT NOT NULL DEFAULT 'coder',
    wave INTEGER NOT NULL DEFAULT 0,
    depends_on TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending',
    isolation_type TEXT NOT NULL DEFAULT 'shared',
    worktree_path TEXT,
    container_id TEXT,
    branch_name TEXT,
    started_at TEXT,
    completed_at TEXT,
    error TEXT
);

CREATE TABLE IF NOT EXISTS agent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES agent_tasks(id),
    event_type TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE pipeline_runs ADD COLUMN team_id INTEGER REFERENCES agent_teams(id);
ALTER TABLE pipeline_runs ADD COLUMN has_team INTEGER NOT NULL DEFAULT 0;
