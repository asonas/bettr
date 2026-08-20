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

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
);

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
    idempotency_key TEXT,
    changed_fields_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX audit_events_project_id ON audit_events(project_id);
CREATE INDEX audit_events_operation ON audit_events(operation);
CREATE INDEX audit_events_finished_at ON audit_events(finished_at);
CREATE INDEX audit_events_idempotency_key ON audit_events(idempotency_key);

CREATE TABLE idempotency_records (
    idempotency_key TEXT PRIMARY KEY,
    operation TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idempotency_records_created_at
    ON idempotency_records(created_at);

CREATE TABLE issue_dependencies (
    id TEXT PRIMARY KEY,
    blocker_issue_id TEXT NOT NULL REFERENCES issues(id),
    blocked_issue_id TEXT NOT NULL REFERENCES issues(id),
    relation TEXT NOT NULL CHECK (relation = 'blocks'),
    created_at TEXT NOT NULL,
    UNIQUE(blocker_issue_id, blocked_issue_id, relation),
    CHECK (blocker_issue_id <> blocked_issue_id)
);

CREATE INDEX issue_dependencies_blocker
    ON issue_dependencies(blocker_issue_id);
CREATE INDEX issue_dependencies_blocked
    ON issue_dependencies(blocked_issue_id);

CREATE TABLE issue_parents (
    child_issue_id TEXT PRIMARY KEY REFERENCES issues(id),
    parent_issue_id TEXT NOT NULL REFERENCES issues(id),
    created_at TEXT NOT NULL,
    CHECK (child_issue_id <> parent_issue_id)
);

CREATE INDEX issue_parents_parent
    ON issue_parents(parent_issue_id);

CREATE TABLE issue_leases (
    issue_id TEXT PRIMARY KEY REFERENCES issues(id),
    agent TEXT NOT NULL,
    session_id TEXT NOT NULL,
    claimed_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    lease_revision INTEGER NOT NULL CHECK (lease_revision > 0)
);

CREATE INDEX issue_leases_expires_at
    ON issue_leases(expires_at);

CREATE TABLE decision_requests (
    id TEXT PRIMARY KEY,
    issue_id TEXT NOT NULL REFERENCES issues(id),
    question TEXT NOT NULL,
    background TEXT NOT NULL,
    requester_kind TEXT,
    requester_name TEXT,
    requester_session_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('open', 'resolved')),
    answer TEXT,
    resolver_kind TEXT,
    resolver_name TEXT,
    resolver_session_id TEXT,
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    blocker TEXT NOT NULL DEFAULT '',
    options_json TEXT NOT NULL DEFAULT '[]',
    recommendation TEXT NOT NULL DEFAULT '',
    resume_condition TEXT NOT NULL DEFAULT ''
);

CREATE INDEX decision_requests_issue_status
    ON decision_requests(issue_id, status);
CREATE INDEX decision_requests_status
    ON decision_requests(status);

PRAGMA application_id = 1112822866;
PRAGMA user_version = 5;
