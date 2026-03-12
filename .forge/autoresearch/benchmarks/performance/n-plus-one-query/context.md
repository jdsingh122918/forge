# N+1 Query Pattern in Dashboard Builder

This is a synthetic benchmark showing the classic N+1 query anti-pattern in a dashboard
builder module. The code constructs a project dashboard by querying projects, then issues
for each project, then pipeline runs for each issue, then events for each run.

With P projects, I issues per project, R runs per issue, and E events per run, the code
issues `1 + P + P*I + P*I*R` individual database queries. For even modest data (10 projects,
20 issues each, 3 runs each), this results in 1 + 10 + 200 + 600 = 811 queries when
4 JOINed queries would suffice.

The fix would be to:
1. Query all projects, issues, runs, and events in bulk (4 queries)
2. Use JOINs or LEFT JOINs to fetch related data in a single query
3. Group results in application code using HashMaps keyed by parent IDs
4. Add pagination to avoid loading entire tables into memory
