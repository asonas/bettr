CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE TABLE issues (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    state TEXT NOT NULL CHECK (state IN ('todo', 'in_progress', 'blocked', 'done', 'cancelled')),
    priority TEXT,
    assignee_kind TEXT,
    assignee_name TEXT,
    revision INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, number)
);

CREATE INDEX issues_project_state_updated_at
    ON issues(project_id, state, updated_at);

CREATE TABLE comments (
    id TEXT PRIMARY KEY,
    issue_id TEXT NOT NULL REFERENCES issues(id),
    body TEXT NOT NULL,
    author_kind TEXT,
    author_name TEXT,
    created_at TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE domain_events (
    id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE,
    project_id TEXT REFERENCES projects(id),
    issue_id TEXT REFERENCES issues(id),
    event_type TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE INDEX domain_events_sequence ON domain_events(sequence);

CREATE TABLE audit_events (
    id TEXT PRIMARY KEY,
    occurred_at TEXT NOT NULL,
    operation TEXT NOT NULL,
    success INTEGER NOT NULL CHECK (success IN (0, 1)),
    initiator_kind TEXT,
    initiator_name TEXT,
    session_id TEXT,
    target_type TEXT,
    target_id TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

PRAGMA user_version = 1;
