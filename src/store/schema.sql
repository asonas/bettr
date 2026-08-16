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
    priority TEXT CHECK (
        priority IS NULL OR priority IN ('critical', 'high', 'medium', 'low')
    ),
    assignee_kind TEXT,
    assignee_name TEXT,
    revision INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, number)
);

CREATE INDEX issues_project_state_updated_at
    ON issues(project_id, state, updated_at);

CREATE INDEX issues_state ON issues(state);
CREATE INDEX issues_priority ON issues(priority);
CREATE INDEX issues_assignee_name ON issues(assignee_name);
CREATE INDEX issues_updated_at ON issues(updated_at);

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
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    operation TEXT NOT NULL,
    success INTEGER NOT NULL CHECK (success IN (0, 1)),
    exit_code INTEGER NOT NULL CHECK (exit_code BETWEEN 0 AND 255),
    initiator_kind TEXT,
    initiator_name TEXT,
    session_id TEXT,
    project_id TEXT,
    project_name TEXT,
    target_type TEXT,
    target_id TEXT,
    revision INTEGER,
    changed_fields_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX audit_events_project_id ON audit_events(project_id);
CREATE INDEX audit_events_operation ON audit_events(operation);
CREATE INDEX audit_events_finished_at ON audit_events(finished_at);

PRAGMA application_id = 1112822866;
PRAGMA user_version = 1;
