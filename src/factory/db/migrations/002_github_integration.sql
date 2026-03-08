ALTER TABLE projects ADD COLUMN github_repo TEXT;
ALTER TABLE issues ADD COLUMN github_issue_number INTEGER;
CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_github_number
    ON issues(project_id, github_issue_number)
    WHERE github_issue_number IS NOT NULL;
