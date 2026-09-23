-- Every media item uploaded into a project, in upload order. A registry for
-- listing, membership checks and /data lookups: appended by POST /sources and
-- never shrunk by an undo. What is on the timeline is the fold's doc.sources.
CREATE TABLE project_sources (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    media_id   TEXT NOT NULL,
    start_at   REAL NOT NULL, -- stitched offset (OFFSET is an SQL keyword)
    duration   REAL NOT NULL,
    PRIMARY KEY (project_id, position)
);
CREATE INDEX project_sources_media ON project_sources(media_id);

-- Source 0 is the project's own media. Its timing lives in meta.json, which
-- SQL cannot read, so the row records zeros; nothing reads them back.
INSERT INTO project_sources (project_id, position, media_id, start_at, duration)
SELECT id, 0, media_id, 0.0, 0.0 FROM projects;
