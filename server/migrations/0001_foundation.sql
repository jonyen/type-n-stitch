CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    color         TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE projects (
    id         TEXT PRIMARY KEY,
    media_id   TEXT NOT NULL,
    owner_id   TEXT NOT NULL REFERENCES users(id),
    title      TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX projects_media ON projects(media_id);

CREATE TABLE project_members (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role       TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'commenter', 'viewer')),
    PRIMARY KEY (project_id, user_id)
);

CREATE TABLE edit_ops (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    op_id      TEXT NOT NULL,
    author_id  TEXT NOT NULL REFERENCES users(id),
    op         TEXT NOT NULL,
    undone_by  INTEGER,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, seq),
    UNIQUE (project_id, op_id)
);
