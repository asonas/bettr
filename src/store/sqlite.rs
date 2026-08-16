pub struct Database {
    connection: rusqlite::Connection,
}

const BETTR_APPLICATION_ID: u32 = 0x4254_5452;
const BETTR_SCHEMA_VERSION: u32 = 1;

struct DatabaseIdentity {
    application_id: u32,
    user_version: u32,
}

impl DatabaseIdentity {
    fn is_current_bettr(&self) -> bool {
        self.application_id == BETTR_APPLICATION_ID && self.user_version == BETTR_SCHEMA_VERSION
    }
}

fn read_sqlite_header_identity(
    path: &std::path::Path,
) -> Result<DatabaseIdentity, crate::error::AppError> {
    use std::io::Read as _;

    let metadata =
        std::fs::metadata(path).map_err(|_| crate::error::AppError::DatabaseNotInitialized)?;
    if !metadata.is_file() {
        return Err(crate::error::AppError::DatabaseNotInitialized);
    }

    let mut header = [0_u8; 100];
    let mut file =
        std::fs::File::open(path).map_err(|_| crate::error::AppError::DatabaseNotInitialized)?;
    file.read_exact(&mut header)
        .map_err(|_| crate::error::AppError::DatabaseNotInitialized)?;
    if &header[..16] != b"SQLite format 3\0" {
        return Err(crate::error::AppError::DatabaseNotInitialized);
    }

    Ok(DatabaseIdentity {
        user_version: u32::from_be_bytes(header[60..64].try_into().unwrap()),
        application_id: u32::from_be_bytes(header[68..72].try_into().unwrap()),
    })
}

struct AuditInsert<'a> {
    operation: &'a str,
    success: bool,
    context: &'a crate::domain::ExecutionContext,
    target: Option<(&'a str, &'a str)>,
    project: Option<(uuid::Uuid, &'a str)>,
    revision: Option<i64>,
    started_at: chrono::DateTime<chrono::Utc>,
    exit_code: u8,
    changed_fields: &'a [&'a str],
    metadata_json: &'a str,
}

#[derive(Default)]
pub(crate) struct AuditSubject {
    project: Option<(uuid::Uuid, String)>,
    target: Option<(String, String)>,
    revision: Option<i64>,
}

impl AuditSubject {
    pub(crate) fn issue(
        project_id: uuid::Uuid,
        project_name: &str,
        issue_id: uuid::Uuid,
        revision: i64,
    ) -> Self {
        Self {
            project: Some((project_id, project_name.to_owned())),
            target: Some(("issue".to_owned(), issue_id.to_string())),
            revision: Some(revision),
        }
    }

    pub(crate) fn project(&self) -> Option<(uuid::Uuid, &str)> {
        self.project
            .as_ref()
            .map(|(project_id, project_name)| (*project_id, project_name.as_str()))
    }
}

pub(crate) struct AuditedResult<T> {
    pub result: Result<T, crate::error::AppError>,
    pub subject: AuditSubject,
}

impl Database {
    pub fn initialize(
        path: &std::path::Path,
        context: &crate::domain::ExecutionContext,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, crate::error::AppError> {
        Self::create_parent_directory(path)?;

        let file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let error = crate::error::AppError::DatabaseAlreadyInitialized;
                if Self::is_initialized_database(path).unwrap_or(false) {
                    let mut database = Self::open(path)?;
                    database.record_failed_operation(
                        "init",
                        context,
                        &error,
                        &AuditSubject::default(),
                        started_at,
                    )?;
                }
                return Err(error);
            }
            Err(error) => return Err(crate::error::AppError::Internal(error.to_string())),
        };
        drop(file);

        let database = Self::open_for_initialization(path);
        let mut database = match database {
            Ok(database) => database,
            Err(error) => {
                return Err(Self::cleanup_after_initialization_failure(path, error));
            }
        };

        if let Err(error) = database.initialize_schema(context, started_at) {
            drop(database);
            return Err(Self::cleanup_after_initialization_failure(path, error));
        }

        Ok(database)
    }

    pub fn open(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        if !read_sqlite_header_identity(path)?.is_current_bettr() {
            return Err(crate::error::AppError::DatabaseNotInitialized);
        }

        Self::open_verified(path)
    }

    #[allow(dead_code)]
    pub const fn connection(&self) -> &rusqlite::Connection {
        &self.connection
    }

    pub fn create_project(
        &mut self,
        name: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Project, crate::error::AppError> {
        let project = crate::domain::Project {
            id: uuid::Uuid::new_v4(),
            name: name.to_owned(),
            archived: false,
            created_at: chrono::Utc::now(),
        };
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;

        match transaction.execute(
            "INSERT INTO projects (id, name, archived, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                project.id.to_string(),
                project.name,
                i64::from(project.archived),
                project.created_at.to_rfc3339(),
            ],
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(code, _))
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                return Err(crate::error::AppError::ProjectNameConflict);
            }
            Err(error) => return Err(crate::error::AppError::from(error)),
        }

        let metadata_json =
            Self::event_metadata(serde_json::json!({ "project_name": project.name }), context)?;
        transaction
            .execute(
                "INSERT INTO domain_events (id, sequence, project_id, event_type, metadata_json, created_at)
                 VALUES (?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events), ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    project.id.to_string(),
                    "project_created",
                    metadata_json,
                    project.created_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let project_id = project.id.to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "project_create",
                success: true,
                context,
                target: Some(("project", &project_id)),
                project: Some((project.id, &project.name)),
                revision: None,
                started_at: project.created_at,
                exit_code: 0,
                changed_fields: &["name"],
                metadata_json: "{}",
            },
        )?;
        transaction.commit().map_err(crate::error::AppError::from)?;

        Ok(project)
    }

    pub fn list_projects(&self) -> Result<Vec<crate::domain::Project>, crate::error::AppError> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, archived, created_at FROM projects ORDER BY name ASC")
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement.query([]).map_err(crate::error::AppError::from)?;
        let mut projects = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            let id: String = row.get(0).map_err(crate::error::AppError::from)?;
            let created_at: String = row.get(3).map_err(crate::error::AppError::from)?;
            projects.push(crate::domain::Project {
                id: uuid::Uuid::parse_str(&id)
                    .map_err(|error| crate::error::AppError::Internal(error.to_string()))?,
                name: row.get(1).map_err(crate::error::AppError::from)?,
                archived: row.get::<_, i64>(2).map_err(crate::error::AppError::from)? != 0,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map_err(|error| crate::error::AppError::Internal(error.to_string()))?
                    .with_timezone(&chrono::Utc),
            });
        }
        Ok(projects)
    }

    pub fn create_issue(
        &mut self,
        project_name: &str,
        input: &crate::domain::NewIssue,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        self.create_issue_once(project_name, input, context)
    }

    pub fn show_issue(
        &self,
        project_name: &str,
        number: i64,
    ) -> AuditedResult<crate::domain::Issue> {
        let project_id = match self.project_id(project_name) {
            Ok(project_id) => project_id,
            Err(error) => {
                return AuditedResult {
                    result: Err(error),
                    subject: AuditSubject::default(),
                };
            }
        };
        let mut subject = AuditSubject {
            project: Some((project_id, project_name.to_owned())),
            target: None,
            revision: None,
        };
        let mut statement = match self.connection.prepare(
            "SELECT id, project_id, number, title, body, state, priority, assignee_kind,
                        assignee_name, revision, created_at, updated_at
                 FROM issues WHERE project_id = ?1 AND number = ?2",
        ) {
            Ok(statement) => statement,
            Err(error) => {
                return AuditedResult {
                    result: Err(crate::error::AppError::from(error)),
                    subject,
                };
            }
        };
        let result = statement
            .query_row(
                rusqlite::params![project_id.to_string(), number],
                Self::issue_from_row,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("issue not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            });
        if let Ok(issue) = &result {
            subject.target = Some(("issue".to_owned(), issue.id.to_string()));
            subject.revision = Some(issue.revision);
        }
        AuditedResult { result, subject }
    }

    pub fn transition_issue(
        &mut self,
        issue: &crate::domain::Issue,
        expected_revision: i64,
        transition: &crate::domain::Transition,
        target_state: crate::domain::IssueState,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        let updated_at = chrono::Utc::now();
        let changed = transaction
            .execute(
                "UPDATE issues
                 SET state = ?1, updated_at = ?2, revision = revision + 1
                 WHERE id = ?3 AND revision = ?4",
                rusqlite::params![
                    target_state.as_str(),
                    updated_at.to_rfc3339(),
                    issue.id.to_string(),
                    expected_revision,
                ],
            )
            .map_err(crate::error::AppError::from)?;
        if changed == 0 {
            let current_revision = transaction
                .query_row(
                    "SELECT revision FROM issues WHERE id = ?1",
                    [issue.id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        crate::error::AppError::NotFound("issue not found".to_owned())
                    }
                    error => crate::error::AppError::from(error),
                })?;
            return Err(crate::error::AppError::RevisionConflict { current_revision });
        }

        let mut updated_issue = issue.clone();
        updated_issue.state = target_state;
        updated_issue.revision = expected_revision + 1;
        updated_issue.updated_at = updated_at;
        let event_metadata = Self::event_metadata(
            transition.event_metadata(issue.state, target_state, updated_issue.revision),
            context,
        )?;
        transaction
            .execute(
                "INSERT INTO domain_events (
                    id, sequence, project_id, issue_id, event_type, metadata_json, created_at
                 ) VALUES (
                    ?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),
                    ?2, ?3, ?4, ?5, ?6
                 )",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    updated_issue.project_id.to_string(),
                    updated_issue.id.to_string(),
                    transition.event_type(),
                    event_metadata,
                    updated_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let issue_id = updated_issue.id.to_string();
        let project_name =
            Self::project_name_in_transaction(&transaction, updated_issue.project_id)?;
        let audit_metadata = serde_json::json!({ "revision": updated_issue.revision }).to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: transition.operation(),
                success: true,
                context,
                target: Some(("issue", &issue_id)),
                project: Some((updated_issue.project_id, &project_name)),
                revision: Some(updated_issue.revision),
                started_at: updated_at,
                exit_code: 0,
                changed_fields: transition.changed_fields(),
                metadata_json: &audit_metadata,
            },
        )?;
        transaction.commit().map_err(crate::error::AppError::from)?;

        Ok(updated_issue)
    }

    pub fn update_issue(
        &mut self,
        issue: &crate::domain::Issue,
        expected_revision: i64,
        patch: &crate::domain::IssuePatch,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        let updated_at = chrono::Utc::now();
        let mut updated_issue = issue.clone();
        patch.apply_to(&mut updated_issue);
        updated_issue.revision = expected_revision + 1;
        updated_issue.updated_at = updated_at;
        let changed = transaction
            .execute(
                "UPDATE issues
                 SET title = ?1, body = ?2, priority = ?3, assignee_kind = ?4,
                     assignee_name = ?5, updated_at = ?6, revision = revision + 1
                 WHERE id = ?7 AND revision = ?8",
                rusqlite::params![
                    updated_issue.title,
                    updated_issue.body,
                    updated_issue.priority.map(crate::domain::Priority::as_str),
                    updated_issue
                        .assignee_kind
                        .map(crate::domain::AssigneeKind::as_str),
                    updated_issue.assignee_name,
                    updated_at.to_rfc3339(),
                    issue.id.to_string(),
                    expected_revision,
                ],
            )
            .map_err(crate::error::AppError::from)?;
        if changed == 0 {
            let current_revision = transaction
                .query_row(
                    "SELECT revision FROM issues WHERE id = ?1",
                    [issue.id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        crate::error::AppError::NotFound("issue not found".to_owned())
                    }
                    error => crate::error::AppError::from(error),
                })?;
            return Err(crate::error::AppError::RevisionConflict { current_revision });
        }

        let event_metadata =
            Self::event_metadata(patch.event_metadata(updated_issue.revision), context)?;
        transaction
            .execute(
                "INSERT INTO domain_events (
                    id, sequence, project_id, issue_id, event_type, metadata_json, created_at
                 ) VALUES (
                    ?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),
                    ?2, ?3, 'issue_updated', ?4, ?5
                 )",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    updated_issue.project_id.to_string(),
                    updated_issue.id.to_string(),
                    event_metadata,
                    updated_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let issue_id = updated_issue.id.to_string();
        let project_name =
            Self::project_name_in_transaction(&transaction, updated_issue.project_id)?;
        let audit_metadata = serde_json::json!({ "revision": updated_issue.revision }).to_string();
        let changed_fields = patch.changed_fields(issue, &updated_issue);
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_edit",
                success: true,
                context,
                target: Some(("issue", &issue_id)),
                project: Some((updated_issue.project_id, &project_name)),
                revision: Some(updated_issue.revision),
                started_at: updated_at,
                exit_code: 0,
                changed_fields: &changed_fields,
                metadata_json: &audit_metadata,
            },
        )?;
        transaction.commit().map_err(crate::error::AppError::from)?;

        Ok(updated_issue)
    }

    pub fn add_comment(
        &mut self,
        project_name: &str,
        number: i64,
        body: &str,
        context: &crate::domain::ExecutionContext,
    ) -> AuditedResult<crate::domain::Comment> {
        let mut subject = AuditSubject::default();
        let result = (|| {
            let transaction = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(crate::error::AppError::from)?;
            let project_id = Self::project_id_in_transaction(&transaction, project_name)?;
            subject.project = Some((project_id, project_name.to_owned()));
            let issue = Self::issue_in_transaction(&transaction, project_id, number)?;
            subject.target = Some(("issue".to_owned(), issue.id.to_string()));
            subject.revision = Some(issue.revision);
            let comment = crate::domain::Comment {
                id: uuid::Uuid::new_v4(),
                issue_id: issue.id,
                body: body.to_owned(),
                context: context.clone(),
                created_at: chrono::Utc::now(),
            };
            transaction
                .execute(
                    "INSERT INTO comments (
                        id, issue_id, body, author_kind, author_name, created_at, metadata_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        comment.id.to_string(),
                        comment.issue_id.to_string(),
                        comment.body,
                        context.kind.as_str(),
                        context.initiator_name(),
                        comment.created_at.to_rfc3339(),
                        serde_json::json!({ "session_id": context.session_id }).to_string(),
                    ],
                )
                .map_err(crate::error::AppError::from)?;
            transaction
                .execute(
                    "UPDATE issues SET updated_at = ?1 WHERE id = ?2",
                    rusqlite::params![
                        comment.created_at.to_rfc3339(),
                        comment.issue_id.to_string()
                    ],
                )
                .map_err(crate::error::AppError::from)?;
            let event_metadata = Self::event_metadata(
                serde_json::json!({ "comment_id": comment.id, "body": comment.body }),
                context,
            )?;
            transaction
                .execute(
                    "INSERT INTO domain_events (
                        id, sequence, project_id, issue_id, event_type, metadata_json, created_at
                     ) VALUES (
                        ?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),
                        ?2, ?3, 'comment_added', ?4, ?5
                     )",
                    rusqlite::params![
                        uuid::Uuid::new_v4().to_string(),
                        issue.project_id.to_string(),
                        comment.issue_id.to_string(),
                        event_metadata,
                        comment.created_at.to_rfc3339(),
                    ],
                )
                .map_err(crate::error::AppError::from)?;
            let issue_id = issue.id.to_string();
            let audit_metadata = serde_json::json!({
                "comment_id": comment.id,
                "revision": issue.revision,
            })
            .to_string();
            Self::insert_audit_event(
                &transaction,
                AuditInsert {
                    operation: "issue_comment",
                    success: true,
                    context,
                    target: Some(("issue", &issue_id)),
                    project: Some((issue.project_id, project_name)),
                    revision: Some(issue.revision),
                    started_at: comment.created_at,
                    exit_code: 0,
                    changed_fields: &["comment", "updated_at"],
                    metadata_json: &audit_metadata,
                },
            )?;
            transaction.commit().map_err(crate::error::AppError::from)?;
            Ok(comment)
        })();
        AuditedResult { result, subject }
    }

    pub fn issue_history(
        &mut self,
        project_name: &str,
        number: i64,
    ) -> AuditedResult<(crate::domain::Issue, Vec<crate::domain::DomainEvent>)> {
        let mut subject = AuditSubject::default();
        let result = (|| {
            let transaction = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
                .map_err(crate::error::AppError::from)?;
            let project_id = Self::project_id_in_transaction(&transaction, project_name)?;
            subject.project = Some((project_id, project_name.to_owned()));
            let issue = Self::issue_in_transaction(&transaction, project_id, number)?;
            subject.target = Some(("issue".to_owned(), issue.id.to_string()));
            subject.revision = Some(issue.revision);
            let events = Self::issue_history_in_transaction(&transaction, issue.id)?;
            transaction.commit().map_err(crate::error::AppError::from)?;
            Ok((issue, events))
        })();
        AuditedResult { result, subject }
    }

    fn issue_history_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        issue_id: uuid::Uuid,
    ) -> Result<Vec<crate::domain::DomainEvent>, crate::error::AppError> {
        let mut statement = transaction
            .prepare(
                "SELECT sequence, event_type, metadata_json, created_at
                 FROM domain_events WHERE issue_id = ?1 ORDER BY sequence ASC",
            )
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query([issue_id.to_string()])
            .map_err(crate::error::AppError::from)?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            let metadata_json: String = row.get(2).map_err(crate::error::AppError::from)?;
            let mut metadata: serde_json::Value = serde_json::from_str(&metadata_json)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
            let context = metadata
                .as_object_mut()
                .and_then(|metadata| metadata.remove("context"))
                .ok_or_else(|| {
                    crate::error::AppError::Internal(
                        "domain event is missing execution context".to_owned(),
                    )
                })?;
            let context = serde_json::from_value(context)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
            let revision = metadata.get("revision").and_then(serde_json::Value::as_i64);
            let created_at: String = row.get(3).map_err(crate::error::AppError::from)?;
            events.push(crate::domain::DomainEvent {
                sequence: row.get(0).map_err(crate::error::AppError::from)?,
                event_type: row.get(1).map_err(crate::error::AppError::from)?,
                revision,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map_err(|error| crate::error::AppError::Internal(error.to_string()))?
                    .with_timezone(&chrono::Utc),
                context,
                metadata,
            });
        }
        Ok(events)
    }

    pub fn list_issues(
        &self,
        filter: &crate::domain::IssueFilter,
    ) -> Result<Vec<crate::domain::IssueListItem>, crate::error::AppError> {
        for project in &filter.projects {
            self.project_id(project)?;
        }

        let mut sql = String::from(
            "SELECT p.name, i.id, i.project_id, i.number, i.title, i.body, i.state,
                    i.priority, i.assignee_kind, i.assignee_name, i.revision,
                    i.created_at, i.updated_at
             FROM issues i
             JOIN projects p ON p.id = i.project_id
             WHERE 1 = 1",
        );
        let mut parameters = Vec::<rusqlite::types::Value>::new();

        Self::append_text_filter(&mut sql, &mut parameters, "p.name", &filter.projects);
        if filter.states.is_empty() {
            if !filter.include_done {
                sql.push_str(" AND i.state NOT IN ('done', 'cancelled')");
            }
        } else {
            let states = filter
                .states
                .iter()
                .map(|state| state.as_str().to_owned())
                .collect::<Vec<_>>();
            Self::append_text_filter(&mut sql, &mut parameters, "i.state", &states);
        }
        if !filter.priorities.is_empty() {
            let priorities = filter
                .priorities
                .iter()
                .map(|priority| priority.as_str().to_owned())
                .collect::<Vec<_>>();
            Self::append_text_filter(&mut sql, &mut parameters, "i.priority", &priorities);
        }
        if let Some(assignee) = &filter.assignee {
            sql.push_str(" AND i.assignee_name = ?");
            parameters.push(assignee.clone().into());
        }
        if let Some(updated_after) = filter.updated_after {
            sql.push_str(" AND i.updated_at > ?");
            parameters.push(updated_after.to_rfc3339().into());
        }
        if let Some(query) = &filter.query {
            let pattern = format!("%{}%", Self::escape_like(query));
            sql.push_str(
                " AND (i.title LIKE ? ESCAPE '\\' OR COALESCE(i.body, '') LIKE ? ESCAPE '\\')",
            );
            parameters.push(pattern.clone().into());
            parameters.push(pattern.into());
        }
        sql.push_str(
            " ORDER BY
                CASE WHEN 0 THEN 0 ELSE 1 END ASC,
                CASE i.state WHEN 'blocked' THEN 0 WHEN 'in_progress' THEN 1 ELSE 2 END ASC,
                CASE i.priority
                    WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2
                    WHEN 'low' THEN 3 ELSE 4
                END ASC,
                i.created_at ASC,
                p.name ASC,
                i.number ASC",
        );

        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(parameters.iter()))
            .map_err(crate::error::AppError::from)?;
        let mut issues = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            issues.push(crate::domain::IssueListItem {
                project: row.get(0).map_err(crate::error::AppError::from)?,
                issue: Self::issue_from_row_at(row, 1).map_err(crate::error::AppError::from)?,
            });
        }
        Ok(issues)
    }

    pub fn list_audit_events(
        &self,
        filter: &crate::app::AuditFilter,
    ) -> Result<Vec<crate::app::AuditEvent>, crate::error::AppError> {
        let mut sql = String::from(
            "SELECT id, COALESCE(started_at, occurred_at),
                    COALESCE(finished_at, occurred_at), operation, success,
                    COALESCE(exit_code, CASE WHEN success = 1 THEN 0 ELSE 10 END),
                    initiator_kind, initiator_name, session_id, project_id, project_name,
                    target_type, target_id, revision, changed_fields_json
             FROM audit_events WHERE 1 = 1",
        );
        let mut parameters = Vec::<rusqlite::types::Value>::new();
        if let Some(project_id) = filter.project_id {
            sql.push_str(" AND project_id = ?");
            parameters.push(project_id.to_string().into());
        }
        if let Some(operation) = &filter.operation {
            sql.push_str(" AND operation = ?");
            parameters.push(operation.clone().into());
        }
        if let Some(outcome) = &filter.outcome {
            sql.push_str(" AND success = ?");
            parameters.push(i64::from(outcome == "success").into());
        }
        if let Some(kind) = &filter.kind {
            sql.push_str(" AND initiator_kind = ?");
            parameters.push(kind.clone().into());
        }
        if let Some(agent) = &filter.agent {
            sql.push_str(" AND initiator_kind = 'agent' AND initiator_name = ?");
            parameters.push(agent.clone().into());
        }
        if let Some(session_id) = &filter.session_id {
            sql.push_str(" AND session_id = ?");
            parameters.push(session_id.clone().into());
        }
        if let Some(after) = filter.after {
            sql.push_str(" AND COALESCE(finished_at, occurred_at) >= ?");
            parameters.push(after.to_rfc3339().into());
        }
        if let Some(before) = filter.before {
            sql.push_str(" AND COALESCE(started_at, occurred_at) <= ?");
            parameters.push(before.to_rfc3339().into());
        }
        sql.push_str(" ORDER BY COALESCE(finished_at, occurred_at), rowid");

        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(parameters.iter()))
            .map_err(crate::error::AppError::from)?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            events.push(Self::audit_event_from_row(row)?);
        }
        Ok(events)
    }

    pub fn record_successful_operation(
        &mut self,
        operation: &str,
        context: &crate::domain::ExecutionContext,
        subject: &AuditSubject,
        changed_fields: &[&str],
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation,
                success: true,
                context,
                target: subject
                    .target
                    .as_ref()
                    .map(|(target_type, target_id)| (target_type.as_str(), target_id.as_str())),
                project: subject.project(),
                revision: subject.revision,
                started_at,
                exit_code: 0,
                changed_fields,
                metadata_json: "{}",
            },
        )?;
        transaction.commit().map_err(crate::error::AppError::from)
    }

    pub fn record_failed_operation(
        &mut self,
        operation: &str,
        context: &crate::domain::ExecutionContext,
        error: &crate::error::AppError,
        subject: &AuditSubject,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), crate::error::AppError> {
        // A failure audit uses the same writer lock, so it cannot be persisted while
        // the original operation is reporting that lock as busy. Preserve the
        // actionable busy error instead of waiting again and replacing it.
        if matches!(error, crate::error::AppError::DatabaseBusy(_)) {
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        let metadata_json = serde_json::json!({ "error_code": error.code() }).to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation,
                success: false,
                context,
                target: subject
                    .target
                    .as_ref()
                    .map(|(target_type, target_id)| (target_type.as_str(), target_id.as_str())),
                project: subject
                    .project
                    .as_ref()
                    .map(|(project_id, project_name)| (*project_id, project_name.as_str())),
                revision: match error {
                    crate::error::AppError::RevisionConflict { current_revision } => {
                        Some(*current_revision)
                    }
                    _ => subject.revision,
                },
                started_at,
                exit_code: error.exit_code() as u8,
                changed_fields: &[],
                metadata_json: &metadata_json,
            },
        )?;
        transaction.commit().map_err(crate::error::AppError::from)
    }

    fn insert_audit_event(
        transaction: &rusqlite::Transaction<'_>,
        event: AuditInsert<'_>,
    ) -> Result<(), crate::error::AppError> {
        let (target_type, target_id) = event.target.unzip();
        let (project_id, project_name) = event
            .project
            .map(|(id, name)| (Some(id.to_string()), Some(name)))
            .unwrap_or((None, None));
        let finished_at = chrono::Utc::now();
        let changed_fields_json = Self::changed_fields_json(event.operation, event.changed_fields)?;
        transaction
            .execute(
                "INSERT INTO audit_events (
                    id, occurred_at, started_at, finished_at, operation, success, exit_code,
                    initiator_kind, initiator_name, session_id, project_id, project_name,
                    target_type, target_id, revision, changed_fields_json, metadata_json
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
                 )",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    finished_at.to_rfc3339(),
                    event.started_at.to_rfc3339(),
                    finished_at.to_rfc3339(),
                    event.operation,
                    i64::from(event.success),
                    event.exit_code,
                    event.context.kind.as_str(),
                    event.context.initiator_name(),
                    event.context.session_id,
                    project_id,
                    project_name,
                    target_type,
                    target_id,
                    event.revision,
                    changed_fields_json,
                    event.metadata_json,
                ],
            )
            .map_err(crate::error::AppError::from)?;
        Ok(())
    }

    pub(crate) fn project_audit_subject(&self, project_name: &str) -> AuditSubject {
        let project_id = match self.project_id(project_name) {
            Ok(project_id) => project_id,
            Err(_) => return AuditSubject::default(),
        };
        AuditSubject {
            project: Some((project_id, project_name.to_owned())),
            target: None,
            revision: None,
        }
    }

    fn audit_event_from_row(
        row: &rusqlite::Row<'_>,
    ) -> Result<crate::app::AuditEvent, crate::error::AppError> {
        let parse_uuid = |value: String| {
            uuid::Uuid::parse_str(&value)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))
        };
        let parse_timestamp = |value: String| {
            chrono::DateTime::parse_from_rfc3339(&value)
                .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))
        };
        let operation: String = row.get(3).map_err(crate::error::AppError::from)?;
        let initiator_kind: Option<String> = row.get(6).map_err(crate::error::AppError::from)?;
        let initiator_name: Option<String> = row.get(7).map_err(crate::error::AppError::from)?;
        let session_id: Option<String> = row.get(8).map_err(crate::error::AppError::from)?;
        let context = match initiator_kind.as_deref().unwrap_or("system") {
            "agent" => crate::domain::ExecutionContext {
                kind: crate::domain::InitiatorKind::Agent,
                agent: initiator_name,
                session_id,
                operator: None,
            },
            "human" => crate::domain::ExecutionContext {
                kind: crate::domain::InitiatorKind::Human,
                agent: None,
                session_id: None,
                operator: initiator_name,
            },
            "system" => crate::domain::ExecutionContext {
                kind: crate::domain::InitiatorKind::System,
                agent: None,
                session_id: None,
                operator: None,
            },
            kind => {
                return Err(crate::error::AppError::Internal(format!(
                    "audit event has unknown execution kind {kind}"
                )));
            }
        };
        let project_id: Option<String> = row.get(9).map_err(crate::error::AppError::from)?;
        let project_name: Option<String> = row.get(10).map_err(crate::error::AppError::from)?;
        let project = if Self::operation_allows_project(&operation) {
            match project_id.zip(project_name) {
                Some((id, name)) => Some(crate::app::AuditProject {
                    id: parse_uuid(id)?,
                    name,
                }),
                None => None,
            }
        } else {
            None
        };
        let target_type: Option<String> = row.get(11).map_err(crate::error::AppError::from)?;
        let target_id: Option<String> = row.get(12).map_err(crate::error::AppError::from)?;
        let target_parts = Self::allowed_target_kind(&operation).and_then(|allowed_kind| {
            target_type
                .zip(target_id)
                .filter(|(kind, _)| kind == allowed_kind)
        });
        let target = match target_parts {
            Some((kind, id)) => Some(crate::app::AuditTarget {
                kind,
                id: parse_uuid(id)?,
            }),
            None => None,
        };
        let revision = if Self::operation_allows_revision(&operation) {
            row.get(13).map_err(crate::error::AppError::from)?
        } else {
            None
        };
        let success: i64 = row.get(4).map_err(crate::error::AppError::from)?;
        let exit_code: i64 = row.get(5).map_err(crate::error::AppError::from)?;
        let exit_code = u8::try_from(exit_code).map_err(|error| {
            crate::error::AppError::Internal(format!("invalid audit exit code: {error}"))
        })?;
        let changed_fields = Self::audit_changed_fields_from_row(&operation, row, 14)?;
        Ok(crate::app::AuditEvent {
            id: parse_uuid(row.get(0).map_err(crate::error::AppError::from)?)?,
            started_at: parse_timestamp(row.get(1).map_err(crate::error::AppError::from)?)?,
            finished_at: parse_timestamp(row.get(2).map_err(crate::error::AppError::from)?)?,
            operation,
            project,
            target,
            context,
            outcome: if success == 1 { "success" } else { "failure" }.to_owned(),
            exit_code,
            revision,
            changed_fields,
        })
    }

    fn operation_allows_project(operation: &str) -> bool {
        matches!(
            operation,
            "project_create"
                | "issue_create"
                | "issue_show"
                | "issue_list"
                | "issue_edit"
                | "issue_comment"
                | "issue_history"
                | "issue_start"
                | "issue_block"
                | "issue_resume"
                | "issue_complete"
                | "issue_cancel"
                | "issue_reopen"
        )
    }

    fn allowed_target_kind(operation: &str) -> Option<&'static str> {
        match operation {
            "project_create" => Some("project"),
            "issue_create" | "issue_show" | "issue_edit" | "issue_comment" | "issue_history"
            | "issue_start" | "issue_block" | "issue_resume" | "issue_complete"
            | "issue_cancel" | "issue_reopen" => Some("issue"),
            _ => None,
        }
    }

    fn operation_allows_revision(operation: &str) -> bool {
        matches!(
            operation,
            "issue_create"
                | "issue_show"
                | "issue_edit"
                | "issue_comment"
                | "issue_history"
                | "issue_start"
                | "issue_block"
                | "issue_resume"
                | "issue_complete"
                | "issue_cancel"
                | "issue_reopen"
        )
    }

    fn allowed_changed_fields(operation: &str) -> &'static [&'static str] {
        match operation {
            "project_create" => &["name"],
            "issue_create" => &["title", "body", "state", "priority"],
            "issue_edit" => &[
                "title",
                "body",
                "priority",
                "assignee_kind",
                "assignee_name",
            ],
            "issue_comment" => &["comment", "updated_at"],
            "issue_start" | "issue_resume" => &["state"],
            "issue_block" => &["state", "reason", "wait_kind"],
            "issue_complete" => &["state", "summary", "verification"],
            "issue_cancel" | "issue_reopen" => &["state", "reason"],
            _ => &[],
        }
    }

    fn changed_fields_json(
        operation: &str,
        fields: &[&str],
    ) -> Result<String, crate::error::AppError> {
        let allowed = Self::allowed_changed_fields(operation);
        if fields.iter().any(|field| !allowed.contains(field)) {
            return Err(crate::error::AppError::Internal(format!(
                "audit operation {operation} contains a disallowed changed field"
            )));
        }
        serde_json::to_string(fields)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))
    }

    fn audit_changed_fields_from_row(
        operation: &str,
        row: &rusqlite::Row<'_>,
        index: usize,
    ) -> Result<Vec<String>, crate::error::AppError> {
        let encoded: String = row.get(index).map_err(crate::error::AppError::from)?;
        let stored = serde_json::from_str::<Vec<String>>(&encoded)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        let allowed = Self::allowed_changed_fields(operation);
        let mut safe = Vec::new();
        for field in stored {
            if allowed.contains(&field.as_str()) && !safe.contains(&field) {
                safe.push(field);
            }
        }
        Ok(safe)
    }

    fn create_issue_once(
        &mut self,
        project_name: &str,
        input: &crate::domain::NewIssue,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        let project_id = Self::project_id_in_transaction(&transaction, project_name)?;
        let number = transaction
            .query_row(
                "SELECT COALESCE(MAX(number), 0) + 1 FROM issues WHERE project_id = ?1",
                [project_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(crate::error::AppError::from)?;
        let now = chrono::Utc::now();
        let issue = crate::domain::Issue {
            id: uuid::Uuid::new_v4(),
            project_id,
            number,
            title: input.title.clone(),
            body: input.body.clone(),
            state: crate::domain::IssueState::Todo,
            priority: input.priority,
            assignee_kind: None,
            assignee_name: None,
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        transaction
            .execute(
                "INSERT INTO issues (
                    id, project_id, number, title, body, state, priority, assignee_kind,
                    assignee_name, revision, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    issue.id.to_string(),
                    issue.project_id.to_string(),
                    issue.number,
                    issue.title,
                    issue.body,
                    issue.state.as_str(),
                    issue.priority.map(crate::domain::Priority::as_str),
                    issue.assignee_kind.map(crate::domain::AssigneeKind::as_str),
                    issue.assignee_name,
                    issue.revision,
                    issue.created_at.to_rfc3339(),
                    issue.updated_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let event_metadata = Self::event_metadata(
            serde_json::json!({ "number": issue.number, "revision": issue.revision }),
            context,
        )?;
        transaction
            .execute(
                "INSERT INTO domain_events (id, sequence, project_id, issue_id, event_type, metadata_json, created_at)
                 VALUES (?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events), ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    issue.project_id.to_string(),
                    issue.id.to_string(),
                    "issue_created",
                    event_metadata,
                    issue.created_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let issue_id = issue.id.to_string();
        let changed_fields = input.changed_fields();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_create",
                success: true,
                context,
                target: Some(("issue", &issue_id)),
                project: Some((issue.project_id, project_name)),
                revision: Some(issue.revision),
                started_at: now,
                exit_code: 0,
                changed_fields: &changed_fields,
                metadata_json: "{}",
            },
        )?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(issue)
    }

    fn project_id(&self, project_name: &str) -> Result<uuid::Uuid, crate::error::AppError> {
        let id = self
            .connection
            .query_row(
                "SELECT id FROM projects WHERE name = ?1",
                [project_name],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("project not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })?;
        uuid::Uuid::parse_str(&id)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))
    }

    fn project_id_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        project_name: &str,
    ) -> Result<uuid::Uuid, crate::error::AppError> {
        let id = transaction
            .query_row(
                "SELECT id FROM projects WHERE name = ?1",
                [project_name],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("project not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })?;
        uuid::Uuid::parse_str(&id)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))
    }

    fn project_name_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        project_id: uuid::Uuid,
    ) -> Result<String, crate::error::AppError> {
        transaction
            .query_row(
                "SELECT name FROM projects WHERE id = ?1",
                [project_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("project not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })
    }

    fn issue_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        project_id: uuid::Uuid,
        number: i64,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        transaction
            .query_row(
                "SELECT id, project_id, number, title, body, state, priority, assignee_kind,
                        assignee_name, revision, created_at, updated_at
                 FROM issues WHERE project_id = ?1 AND number = ?2",
                rusqlite::params![project_id.to_string(), number],
                Self::issue_from_row,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("issue not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })
    }

    fn issue_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::domain::Issue> {
        Self::issue_from_row_at(row, 0)
    }

    fn issue_from_row_at(
        row: &rusqlite::Row<'_>,
        offset: usize,
    ) -> rusqlite::Result<crate::domain::Issue> {
        let id: String = row.get(offset)?;
        let project_id: String = row.get(offset + 1)?;
        let state: String = row.get(offset + 5)?;
        let priority: Option<String> = row.get(offset + 6)?;
        let assignee_kind: Option<String> = row.get(offset + 7)?;
        let created_at: String = row.get(offset + 10)?;
        let updated_at: String = row.get(offset + 11)?;
        let parse_error = |error: Box<dyn std::error::Error + Send + Sync>| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error)
        };
        Ok(crate::domain::Issue {
            id: uuid::Uuid::parse_str(&id).map_err(|error| parse_error(Box::new(error)))?,
            project_id: uuid::Uuid::parse_str(&project_id)
                .map_err(|error| parse_error(Box::new(error)))?,
            number: row.get(offset + 2)?,
            title: row.get(offset + 3)?,
            body: row.get(offset + 4)?,
            state: crate::domain::IssueState::parse(&state)
                .map_err(|error| parse_error(Box::new(error)))?,
            priority: priority
                .map(|value| crate::domain::Priority::parse(&value))
                .transpose()
                .map_err(|error| parse_error(Box::new(error)))?,
            assignee_kind: assignee_kind
                .map(|value| crate::domain::AssigneeKind::parse(&value))
                .transpose()
                .map_err(|error| parse_error(Box::new(error)))?,
            assignee_name: row.get(offset + 8)?,
            revision: row.get(offset + 9)?,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map_err(|error| parse_error(Box::new(error)))?
                .with_timezone(&chrono::Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)
                .map_err(|error| parse_error(Box::new(error)))?
                .with_timezone(&chrono::Utc),
        })
    }

    fn append_text_filter(
        sql: &mut String,
        parameters: &mut Vec<rusqlite::types::Value>,
        column: &str,
        values: &[String],
    ) {
        if values.is_empty() {
            return;
        }
        sql.push_str(" AND ");
        sql.push_str(column);
        sql.push_str(" IN (");
        sql.push_str(&vec!["?"; values.len()].join(", "));
        sql.push(')');
        parameters.extend(values.iter().cloned().map(rusqlite::types::Value::from));
    }

    fn escape_like(value: &str) -> String {
        value
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    }

    fn event_metadata(
        mut metadata: serde_json::Value,
        context: &crate::domain::ExecutionContext,
    ) -> Result<String, crate::error::AppError> {
        let object = metadata.as_object_mut().ok_or_else(|| {
            crate::error::AppError::Internal("domain event metadata must be an object".to_owned())
        })?;
        object.insert(
            "context".to_owned(),
            serde_json::to_value(context)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?,
        );
        Ok(metadata.to_string())
    }

    fn create_parent_directory(path: &std::path::Path) -> Result<(), crate::error::AppError> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };

        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;

            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(parent)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        }

        #[cfg(not(unix))]
        std::fs::create_dir_all(parent)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;

        Ok(())
    }

    fn open_for_initialization(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        Self::open_read_write(path)
    }

    fn open_verified(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        let connection = Self::open_read_write_connection(path)?;
        if !Self::connection_identity(&connection)?.is_current_bettr() {
            return Err(crate::error::AppError::DatabaseNotInitialized);
        }

        Self::configure_connection(connection)
    }

    fn open_read_write(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        let connection = Self::open_read_write_connection(path)?;
        Self::configure_connection(connection)
    }

    fn open_read_write_connection(
        path: &std::path::Path,
    ) -> Result<rusqlite::Connection, crate::error::AppError> {
        let connection = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(crate::error::AppError::from)?;
        Ok(connection)
    }

    fn configure_connection(
        connection: rusqlite::Connection,
    ) -> Result<Self, crate::error::AppError> {
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(crate::error::AppError::from)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(crate::error::AppError::from)?;

        Ok(Self { connection })
    }

    fn is_initialized_database(path: &std::path::Path) -> Result<bool, crate::error::AppError> {
        Ok(read_sqlite_header_identity(path)?.is_current_bettr())
    }

    fn connection_identity(
        connection: &rusqlite::Connection,
    ) -> Result<DatabaseIdentity, crate::error::AppError> {
        let user_version = connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .map_err(crate::error::AppError::from)?;
        let application_id = connection
            .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
            .map_err(crate::error::AppError::from)?;

        Ok(DatabaseIdentity {
            application_id: u32::try_from(application_id)
                .map_err(|_| crate::error::AppError::DatabaseNotInitialized)?,
            user_version: u32::try_from(user_version)
                .map_err(|_| crate::error::AppError::DatabaseNotInitialized)?,
        })
    }

    fn initialize_schema(
        &mut self,
        context: &crate::domain::ExecutionContext,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        transaction
            .execute_batch(include_str!("schema.sql"))
            .map_err(crate::error::AppError::from)?;
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "init",
                success: true,
                context,
                target: None,
                project: None,
                revision: None,
                started_at,
                exit_code: 0,
                changed_fields: &[],
                metadata_json: "{}",
            },
        )?;
        transaction.commit().map_err(crate::error::AppError::from)
    }

    fn cleanup_after_initialization_failure(
        path: &std::path::Path,
        initialization_error: crate::error::AppError,
    ) -> crate::error::AppError {
        match std::fs::remove_file(path) {
            Ok(()) => initialization_error,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => initialization_error,
            Err(error) => crate::error::AppError::Internal(format!(
                "failed to remove newly created database after initialization failure ({initialization_error}): {error}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    fn context() -> crate::domain::ExecutionContext {
        crate::domain::ExecutionContext {
            kind: crate::domain::InitiatorKind::Agent,
            agent: Some("test-agent".to_owned()),
            session_id: Some("test-session".to_owned()),
            operator: None,
        }
    }

    fn initialized_database() -> (tempfile::TempDir, super::Database) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bettr.db");
        let database = super::Database::initialize(&path, &context(), chrono::Utc::now()).unwrap();
        (directory, database)
    }

    #[test]
    fn cleanup_failure_is_returned_to_the_caller() {
        let directory = tempfile::tempdir().unwrap();
        let error = super::Database::cleanup_after_initialization_failure(
            directory.path(),
            crate::error::AppError::Internal("schema application failed".to_owned()),
        );

        assert!(matches!(
            error,
            crate::error::AppError::Internal(message)
                if message.contains("failed to remove newly created database")
        ));
    }

    #[test]
    fn failure_audit_does_not_attach_an_issue_created_after_the_operation_failed() {
        let (_directory, mut database) = initialized_database();
        database.create_project("bettr", &context()).unwrap();
        let error = crate::error::AppError::NotFound("issue not found".to_owned());
        let operation_lookup = database.show_issue("bettr", 1);
        assert!(matches!(
            operation_lookup.result,
            Err(crate::error::AppError::NotFound(_))
        ));
        let subject = operation_lookup.subject;

        database
            .create_issue(
                "bettr",
                &crate::domain::NewIssue {
                    title: "created after failure".to_owned(),
                    body: None,
                    priority: None,
                },
                &context(),
            )
            .unwrap();
        database
            .record_failed_operation(
                "issue_show",
                &context(),
                &error,
                &subject,
                chrono::Utc::now(),
            )
            .unwrap();

        let target_id = database
            .connection
            .query_row(
                "SELECT target_id FROM audit_events
                 WHERE operation = 'issue_show' AND success = 0
                 ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap();
        assert!(target_id.is_none());
    }

    #[test]
    fn revision_conflict_audit_uses_the_revision_seen_by_the_failed_operation() {
        let (_directory, mut database) = initialized_database();
        database.create_project("bettr", &context()).unwrap();
        let issue = database
            .create_issue(
                "bettr",
                &crate::domain::NewIssue {
                    title: "conflicting issue".to_owned(),
                    body: None,
                    priority: None,
                },
                &context(),
            )
            .unwrap();
        let error = crate::error::AppError::RevisionConflict {
            current_revision: 2,
        };
        let operation_lookup = database.show_issue("bettr", 1);
        assert!(operation_lookup.result.is_ok());
        let subject = operation_lookup.subject;

        database
            .connection
            .execute(
                "UPDATE issues SET revision = 3 WHERE id = ?1",
                [issue.id.to_string()],
            )
            .unwrap();
        database
            .record_failed_operation(
                "issue_edit",
                &context(),
                &error,
                &subject,
                chrono::Utc::now(),
            )
            .unwrap();

        let revision = database
            .connection
            .query_row(
                "SELECT revision FROM audit_events
                 WHERE operation = 'issue_edit' AND success = 0
                 ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap();
        assert_eq!(revision, Some(2));
    }
}
