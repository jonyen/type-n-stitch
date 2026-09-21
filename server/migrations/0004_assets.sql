CREATE TABLE project_assets (
    id         TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('video', 'audio')),
    name       TEXT NOT NULL,
    ext        TEXT NOT NULL,
    duration   REAL NOT NULL,
    width      INTEGER,
    height     INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX project_assets_project ON project_assets(project_id);
