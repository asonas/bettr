pub struct Database {
    connection: rusqlite::Connection,
    pending_idempotency: Option<IdempotencyRequest>,
    audit_enabled: bool,
}

const BETTR_APPLICATION_ID: u32 = 0x4254_5452;
const LEASE_TTL_MINUTES: i64 = 15;

struct DatabaseIdentity {
    application_id: u32,
    user_version: u32,
}

impl DatabaseIdentity {
    fn is_bettr_application(&self) -> bool {
        self.application_id == BETTR_APPLICATION_ID
    }

    fn is_supported_bettr(&self) -> bool {
        self.is_bettr_application()
            && crate::store::migrations::is_supported_version(self.user_version)
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
    idempotency_key: Option<&'a str>,
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

#[derive(Clone, Debug)]
pub(crate) struct IdempotencyRequest {
    pub(crate) key: String,
    operation: String,
    request_hash: String,
}

impl IdempotencyRequest {
    pub(crate) fn new(
        key: &str,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<Self, crate::error::AppError> {
        crate::domain::validate_idempotency_key(key)?;
        let canonical_payload = Self::canonicalize(payload);
        let encoded = serde_json::to_vec(&canonical_payload)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(encoded);
        let request_hash = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(Self {
            key: key.to_owned(),
            operation: operation.to_owned(),
            request_hash,
        })
    }

    fn canonicalize(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => {
                let mut entries = object.into_iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(&right.0));
                serde_json::Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key, Self::canonicalize(value)))
                        .collect(),
                )
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(Self::canonicalize).collect())
            }
            value => value,
        }
    }
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
    pub(crate) fn with_idempotency<T, F>(
        &mut self,
        request: Option<IdempotencyRequest>,
        action: F,
    ) -> T
    where
        F: FnOnce(&mut Self) -> T,
    {
        self.pending_idempotency = request;
        let result = action(self);
        self.pending_idempotency = None;
        result
    }

    fn take_idempotency(
        &mut self,
        operation: &str,
    ) -> Result<Option<IdempotencyRequest>, crate::error::AppError> {
        let request = self.pending_idempotency.take();
        if request
            .as_ref()
            .is_some_and(|request| request.operation != operation)
        {
            return Err(crate::error::AppError::Internal(
                "idempotency request operation does not match the write operation".to_owned(),
            ));
        }
        Ok(request)
    }

    #[allow(dead_code)]
    pub fn initialize(
        path: &std::path::Path,
        context: &crate::domain::ExecutionContext,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, crate::error::AppError> {
        Self::initialize_with_idempotency(path, context, started_at, None)
    }

    pub fn initialize_with_idempotency(
        path: &std::path::Path,
        context: &crate::domain::ExecutionContext,
        started_at: chrono::DateTime<chrono::Utc>,
        idempotency_key: Option<&str>,
    ) -> Result<Self, crate::error::AppError> {
        let idempotency = idempotency_key
            .map(|key| {
                IdempotencyRequest::new(
                    key,
                    "init",
                    serde_json::json!({
                        "database": path.to_string_lossy(),
                    }),
                )
            })
            .transpose()?;
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
                    if let Some(result) = database.lookup_idempotency(idempotency.as_ref())? {
                        let _: serde_json::Value = result;
                        return Ok(database);
                    }
                    database.record_failed_operation_with_idempotency(
                        "init",
                        context,
                        &error,
                        &AuditSubject::default(),
                        started_at,
                        idempotency.as_ref().map(|request| request.key.as_str()),
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

        if let Err(error) = database.initialize_schema(context, started_at, idempotency.as_ref()) {
            drop(database);
            return Err(Self::cleanup_after_initialization_failure(path, error));
        }

        Ok(database)
    }

    pub fn open(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        let identity = read_sqlite_header_identity(path)?;
        if !identity.is_bettr_application() {
            return Err(crate::error::AppError::DatabaseNotInitialized);
        }
        if !crate::store::migrations::is_supported_version(identity.user_version) {
            return Err(crate::error::AppError::UnsupportedDatabaseSchemaVersion {
                found_version: identity.user_version,
                current_version: crate::store::migrations::LATEST_SCHEMA_VERSION,
            });
        }

        Self::open_verified(path)
    }

    pub(crate) fn open_for_web_read(
        path: &std::path::Path,
    ) -> Result<Self, crate::error::AppError> {
        let mut database = Self::open(path)?;
        database.audit_enabled = false;
        Ok(database)
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
        let idempotency = self.take_idempotency("project_create")?;
        self.create_project_with_idempotency(name, context, idempotency.as_ref())
    }

    pub fn create_project_with_idempotency(
        &mut self,
        name: &str,
        context: &crate::domain::ExecutionContext,
        idempotency: Option<&IdempotencyRequest>,
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

        if let Some(project) = Self::check_idempotency(&transaction, idempotency)? {
            return Ok(project);
        }

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
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
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
        Self::remember_idempotency(&transaction, idempotency, &project)?;
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

    pub fn batch_issues(
        &mut self,
        operations: &[crate::domain::BatchOperation],
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<Vec<crate::domain::BatchResult>, crate::error::AppError> {
        let idempotency = self.take_idempotency("issue_batch")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(results) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(results);
        }
        if operations.is_empty() {
            return Err(crate::error::AppError::InvalidInput(
                "issue batch input must contain at least one operation".to_owned(),
            ));
        }
        let mut results = Vec::with_capacity(operations.len());
        for operation in operations {
            results.push(Self::batch_operation_in_transaction(
                &transaction,
                operation,
                default_project,
                context,
            )?);
        }
        let project = default_project
            .map(|name| Self::project_id_in_transaction(&transaction, name).map(|id| (id, name)))
            .transpose()?;
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_batch",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: None,
                project,
                revision: None,
                started_at: chrono::Utc::now(),
                exit_code: 0,
                changed_fields: &[],
                metadata_json: "{}",
            },
        )?;
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &results)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(results)
    }

    fn batch_operation_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        operation: &crate::domain::BatchOperation,
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::BatchResult, crate::error::AppError> {
        match operation {
            crate::domain::BatchOperation::IssueCreate {
                project,
                title,
                body,
                priority,
            } => {
                let project = Self::batch_project(project.as_deref(), default_project)?;
                let input = crate::domain::NewIssue {
                    title: title.clone(),
                    body: body.clone(),
                    priority: *priority,
                };
                input.validate()?;
                let issue =
                    Self::batch_create_issue_in_transaction(transaction, project, &input, context)?;
                Self::batch_result(operation.operation_name(), issue)
            }
            crate::domain::BatchOperation::IssueEdit {
                project,
                number,
                revision,
                patch,
            } => {
                let project = Self::batch_project(project.as_deref(), default_project)?;
                if *number < 1 {
                    return Err(crate::error::AppError::InvalidInput(
                        "issue number must be positive".to_owned(),
                    ));
                }
                if *revision < 1 {
                    return Err(crate::error::AppError::InvalidInput(
                        "issue revision must be positive".to_owned(),
                    ));
                }
                patch.validate()?;
                let issue = Self::batch_update_issue_in_transaction(
                    transaction,
                    project,
                    *number,
                    *revision,
                    patch,
                    context,
                )?;
                Self::batch_result(operation.operation_name(), issue)
            }
            crate::domain::BatchOperation::IssueComment {
                project,
                number,
                body,
            } => {
                let project = Self::batch_project(project.as_deref(), default_project)?;
                if *number < 1 {
                    return Err(crate::error::AppError::InvalidInput(
                        "issue number must be positive".to_owned(),
                    ));
                }
                crate::domain::validate_comment_body(body)?;
                let comment = Self::batch_comment_in_transaction(
                    transaction,
                    project,
                    *number,
                    body,
                    context,
                )?;
                Self::batch_result(operation.operation_name(), comment)
            }
            crate::domain::BatchOperation::IssueStart {
                project,
                number,
                revision,
            } => Self::batch_transition_result(
                transaction,
                project.as_deref(),
                default_project,
                *number,
                *revision,
                Ok(crate::domain::Transition::Start),
                context,
            )
            .and_then(|issue| Self::batch_result(operation.operation_name(), issue)),
            crate::domain::BatchOperation::IssueBlock {
                project,
                number,
                revision,
                reason,
                wait_kind,
            } => Self::batch_transition_result(
                transaction,
                project.as_deref(),
                default_project,
                *number,
                *revision,
                crate::domain::Transition::block(reason.clone(), *wait_kind),
                context,
            )
            .and_then(|issue| Self::batch_result(operation.operation_name(), issue)),
            crate::domain::BatchOperation::IssueResume {
                project,
                number,
                revision,
            } => Self::batch_transition_result(
                transaction,
                project.as_deref(),
                default_project,
                *number,
                *revision,
                Ok(crate::domain::Transition::Resume),
                context,
            )
            .and_then(|issue| Self::batch_result(operation.operation_name(), issue)),
            crate::domain::BatchOperation::IssueComplete {
                project,
                number,
                revision,
                summary,
                verification,
            } => Self::batch_transition_result(
                transaction,
                project.as_deref(),
                default_project,
                *number,
                *revision,
                crate::domain::Transition::complete(summary.clone(), verification.clone()),
                context,
            )
            .and_then(|issue| Self::batch_result(operation.operation_name(), issue)),
            crate::domain::BatchOperation::IssueCancel {
                project,
                number,
                revision,
                reason,
            } => Self::batch_transition_result(
                transaction,
                project.as_deref(),
                default_project,
                *number,
                *revision,
                crate::domain::Transition::cancel(reason.clone()),
                context,
            )
            .and_then(|issue| Self::batch_result(operation.operation_name(), issue)),
            crate::domain::BatchOperation::IssueReopen {
                project,
                number,
                revision,
                reason,
            } => Self::batch_transition_result(
                transaction,
                project.as_deref(),
                default_project,
                *number,
                *revision,
                crate::domain::Transition::reopen(reason.clone()),
                context,
            )
            .and_then(|issue| Self::batch_result(operation.operation_name(), issue)),
        }
    }

    fn batch_project<'a>(
        project: Option<&'a str>,
        default_project: Option<&'a str>,
    ) -> Result<&'a str, crate::error::AppError> {
        let project = project.or(default_project).ok_or_else(|| {
            crate::error::AppError::InvalidInput(
                "issue batch operation requires a project or --project".to_owned(),
            )
        })?;
        crate::domain::validate_project_name(project)?;
        Ok(project)
    }

    fn batch_result<T: serde::Serialize>(
        operation: &str,
        result: T,
    ) -> Result<crate::domain::BatchResult, crate::error::AppError> {
        Ok(crate::domain::BatchResult {
            operation: operation.to_owned(),
            result: serde_json::to_value(result)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?,
        })
    }

    fn batch_transition_result(
        transaction: &rusqlite::Transaction<'_>,
        project: Option<&str>,
        default_project: Option<&str>,
        number: i64,
        revision: i64,
        transition: Result<crate::domain::Transition, crate::domain::DomainError>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let project = Self::batch_project(project, default_project)?;
        if number < 1 {
            return Err(crate::error::AppError::InvalidInput(
                "issue number must be positive".to_owned(),
            ));
        }
        if revision < 1 {
            return Err(crate::error::AppError::InvalidInput(
                "issue revision must be positive".to_owned(),
            ));
        }
        let issue = Self::batch_transition_in_transaction(
            transaction,
            project,
            number,
            revision,
            transition?,
            context,
        )?;
        Ok(issue)
    }

    fn batch_create_issue_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        project_name: &str,
        input: &crate::domain::NewIssue,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let project_id = Self::project_id_in_transaction(transaction, project_name)?;
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
        let metadata = Self::event_metadata(
            serde_json::json!({ "number": issue.number, "revision": issue.revision }),
            context,
        )?;
        Self::insert_domain_event(transaction, &issue, "issue_created", &metadata, now)?;
        Ok(issue)
    }

    fn batch_update_issue_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        project_name: &str,
        number: i64,
        expected_revision: i64,
        patch: &crate::domain::IssuePatch,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let project_id = Self::project_id_in_transaction(transaction, project_name)?;
        let issue = Self::issue_in_transaction(transaction, project_id, number)?;
        if issue.revision != expected_revision {
            return Err(crate::error::AppError::RevisionConflict {
                current_revision: issue.revision,
            });
        }
        let mut updated_issue = issue.clone();
        patch.apply_to(&mut updated_issue);
        let updated_at = chrono::Utc::now();
        updated_issue.revision += 1;
        updated_issue.updated_at = updated_at;
        transaction
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
        let metadata = Self::event_metadata(patch.event_metadata(updated_issue.revision), context)?;
        Self::insert_domain_event(
            transaction,
            &updated_issue,
            "issue_updated",
            &metadata,
            updated_at,
        )?;
        Ok(updated_issue)
    }

    fn batch_comment_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        project_name: &str,
        number: i64,
        body: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Comment, crate::error::AppError> {
        let project_id = Self::project_id_in_transaction(transaction, project_name)?;
        let issue = Self::issue_in_transaction(transaction, project_id, number)?;
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
        let metadata = Self::event_metadata(
            serde_json::json!({ "comment_id": comment.id, "body": comment.body }),
            context,
        )?;
        Self::insert_domain_event(
            transaction,
            &issue,
            "comment_added",
            &metadata,
            comment.created_at,
        )?;
        Ok(comment)
    }

    fn batch_transition_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        project_name: &str,
        number: i64,
        expected_revision: i64,
        transition: crate::domain::Transition,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let project_id = Self::project_id_in_transaction(transaction, project_name)?;
        let issue = Self::issue_in_transaction(transaction, project_id, number)?;
        if issue.revision != expected_revision {
            return Err(crate::error::AppError::RevisionConflict {
                current_revision: issue.revision,
            });
        }
        let target_state = issue.state.apply(&transition)?;
        if target_state == crate::domain::IssueState::Done {
            let has_open_decision = transaction
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM decision_requests
                         WHERE issue_id = ?1 AND status = 'open'
                     )",
                    [issue.id.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(crate::error::AppError::from)?;
            if has_open_decision {
                return Err(crate::error::AppError::Conflict(
                    "Issue has an unresolved human decision".to_owned(),
                ));
            }
        }
        let updated_at = chrono::Utc::now();
        transaction
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
        if target_state != crate::domain::IssueState::InProgress {
            transaction
                .execute(
                    "DELETE FROM issue_leases WHERE issue_id = ?1",
                    [issue.id.to_string()],
                )
                .map_err(crate::error::AppError::from)?;
        }
        let mut updated_issue = issue.clone();
        updated_issue.state = target_state;
        updated_issue.revision += 1;
        updated_issue.updated_at = updated_at;
        let metadata = Self::event_metadata(
            transition.event_metadata(issue.state, target_state, updated_issue.revision),
            context,
        )?;
        Self::insert_domain_event(
            transaction,
            &updated_issue,
            transition.event_type(),
            &metadata,
            updated_at,
        )?;
        Ok(updated_issue)
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
        let idempotency = self.take_idempotency(transition.operation())?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(issue) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(issue);
        }
        let updated_at = chrono::Utc::now();
        if target_state == crate::domain::IssueState::Done {
            let has_open_decision = transaction
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM decision_requests
                         WHERE issue_id = ?1 AND status = 'open'
                     )",
                    [issue.id.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(crate::error::AppError::from)?;
            if has_open_decision {
                return Err(crate::error::AppError::Conflict(
                    "Issue has an unresolved human decision".to_owned(),
                ));
            }
        }
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
        if target_state != crate::domain::IssueState::InProgress {
            transaction
                .execute(
                    "DELETE FROM issue_leases WHERE issue_id = ?1",
                    [issue.id.to_string()],
                )
                .map_err(crate::error::AppError::from)?;
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
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
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
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &updated_issue)?;
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
        let idempotency = self.take_idempotency("issue_edit")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(issue) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(issue);
        }
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
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
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
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &updated_issue)?;
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
        let idempotency = match self.take_idempotency("issue_comment") {
            Ok(idempotency) => idempotency,
            Err(error) => {
                return AuditedResult {
                    result: Err(error),
                    subject: AuditSubject::default(),
                };
            }
        };
        let mut subject = AuditSubject::default();
        let result = (|| {
            let transaction = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(crate::error::AppError::from)?;
            if let Some(comment) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
                return Ok(comment);
            }
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
                    idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
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
            Self::remember_idempotency(&transaction, idempotency.as_ref(), &comment)?;
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

    pub fn request_decision(
        &mut self,
        project_name: &str,
        number: i64,
        input: &crate::domain::DecisionRequestInput,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::DecisionRequest, crate::error::AppError> {
        let idempotency = self.take_idempotency("decision_request")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(request) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(request);
        }
        let project_id = Self::project_id_in_transaction(&transaction, project_name)?;
        let issue = Self::issue_in_transaction(&transaction, project_id, number)?;
        if matches!(
            issue.state,
            crate::domain::IssueState::Done | crate::domain::IssueState::Cancelled
        ) {
            return Err(crate::error::AppError::Conflict(
                "cannot request a decision for a completed Issue".to_owned(),
            ));
        }

        let lease_owner = match transaction.query_row(
            "SELECT agent, session_id FROM issue_leases WHERE issue_id = ?1",
            [issue.id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ) {
            Ok(owner) => Some(owner),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => return Err(crate::error::AppError::from(error)),
        };
        if let Some((lease_agent, lease_session_id)) = lease_owner {
            let (agent, session_id) = Self::agent_session(context)?;
            if lease_agent != agent || lease_session_id != session_id {
                return Err(crate::error::AppError::Conflict(
                    "lease is owned by another agent session".to_owned(),
                ));
            }
            transaction
                .execute(
                    "DELETE FROM issue_leases WHERE issue_id = ?1 AND agent = ?2 AND session_id = ?3",
                    rusqlite::params![issue.id.to_string(), agent, session_id],
                )
                .map_err(crate::error::AppError::from)?;
        }

        let now = chrono::Utc::now();
        let request_id = uuid::Uuid::new_v4();
        let updated_revision = issue.revision + 1;
        let options_json = serde_json::to_string(&input.options)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        transaction
            .execute(
                "UPDATE issues
                 SET state = 'blocked', revision = revision + 1, updated_at = ?1
                 WHERE id = ?2 AND revision = ?3",
                rusqlite::params![now.to_rfc3339(), issue.id.to_string(), issue.revision],
            )
            .map_err(crate::error::AppError::from)?;
        transaction
            .execute(
                "INSERT INTO decision_requests (
                    id, issue_id, question, background, requester_kind, requester_name,
                    requester_session_id, status, created_at, blocker, options_json,
                    recommendation, resume_condition
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open', ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    request_id.to_string(),
                    issue.id.to_string(),
                    input.question,
                    input.background,
                    context.kind.as_str(),
                    context.initiator_name(),
                    context.session_id,
                    now.to_rfc3339(),
                    input.blocker,
                    options_json,
                    input.recommendation,
                    input.resume_condition,
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let mut updated_issue = issue.clone();
        updated_issue.state = crate::domain::IssueState::Blocked;
        updated_issue.revision = updated_revision;
        updated_issue.updated_at = now;
        let event_metadata = Self::event_metadata(
            serde_json::json!({
                "request_id": request_id,
                "from_state": issue.state,
                "to_state": updated_issue.state,
                "revision": updated_revision,
            }),
            context,
        )?;
        Self::insert_domain_event(
            &transaction,
            &updated_issue,
            "decision_requested",
            &event_metadata,
            now,
        )?;
        let issue_id = updated_issue.id.to_string();
        let audit_metadata = serde_json::json!({
            "request_id": request_id,
            "revision": updated_revision,
        })
        .to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "decision_request",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &issue_id)),
                project: Some((issue.project_id, project_name)),
                revision: Some(updated_revision),
                started_at: now,
                exit_code: 0,
                changed_fields: &["state", "decision_request"],
                metadata_json: &audit_metadata,
            },
        )?;
        let request = crate::domain::DecisionRequest {
            id: request_id,
            issue: format!("{project_name}#{}", issue.number),
            question: input.question.clone(),
            background: input.background.clone(),
            blocker: input.blocker.clone(),
            options: input.options.clone(),
            recommendation: input.recommendation.clone(),
            resume_condition: input.resume_condition.clone(),
            requester_kind: Some(context.kind),
            requester_name: context.initiator_name().map(str::to_owned),
            requester_session_id: context.session_id.clone(),
            status: "open".to_owned(),
            answer: None,
            resolver_kind: None,
            resolver_name: None,
            resolver_session_id: None,
            created_at: now,
            resolved_at: None,
        };
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &request)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(request)
    }

    pub fn list_decisions(
        &self,
        project_name: &str,
        number: i64,
    ) -> Result<Vec<crate::domain::DecisionRequest>, crate::error::AppError> {
        let project_id = self.project_id(project_name)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT request.id, project.name, issue.number, request.question,
                        request.background, request.requester_kind, request.requester_name,
                        request.requester_session_id, request.status, request.answer,
                        request.resolver_kind, request.resolver_name,
                        request.resolver_session_id, request.created_at, request.resolved_at,
                        request.blocker, request.options_json, request.recommendation,
                        request.resume_condition
                 FROM decision_requests request
                 JOIN issues issue ON issue.id = request.issue_id
                 JOIN projects project ON project.id = issue.project_id
                 WHERE issue.project_id = ?1 AND issue.number = ?2
                 ORDER BY request.created_at ASC, request.id ASC",
            )
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query(rusqlite::params![project_id.to_string(), number])
            .map_err(crate::error::AppError::from)?;
        let mut decisions = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            decisions.push(Self::decision_from_row(row).map_err(crate::error::AppError::from)?);
        }
        Ok(decisions)
    }

    pub fn resolve_decision(
        &mut self,
        request_id: uuid::Uuid,
        answer: &str,
        expected_revision: Option<i64>,
        resolution_input: crate::domain::DecisionResolutionInput,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::DecisionRequest, crate::error::AppError> {
        let idempotency = self.take_idempotency("decision_resolve")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(request) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(request);
        }
        let mut request = transaction
            .query_row(
                "SELECT request.id, project.name, issue.number, request.question,
                        request.background, request.requester_kind, request.requester_name,
                        request.requester_session_id, request.status, request.answer,
                        request.resolver_kind, request.resolver_name,
                        request.resolver_session_id, request.created_at, request.resolved_at,
                        request.blocker, request.options_json, request.recommendation,
                        request.resume_condition
                 FROM decision_requests request
                 JOIN issues issue ON issue.id = request.issue_id
                 JOIN projects project ON project.id = issue.project_id
                 WHERE request.id = ?1",
                [request_id.to_string()],
                Self::decision_from_row,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("decision request not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })?;
        if request.status != "open" {
            return Err(crate::error::AppError::Conflict(
                "decision request is already resolved".to_owned(),
            ));
        }
        if context.kind == crate::domain::InitiatorKind::Agent
            && request.requester_kind == Some(crate::domain::InitiatorKind::Agent)
            && request.requester_name == context.agent
            && request.requester_session_id == context.session_id
        {
            return Err(crate::error::AppError::Conflict(
                "the requesting agent session cannot resolve its own decision request".to_owned(),
            ));
        }
        if context.kind != crate::domain::InitiatorKind::Human {
            return Err(crate::error::AppError::Conflict(
                "decision requests must be resolved by a human".to_owned(),
            ));
        }
        if resolution_input.target_state() == crate::domain::IssueState::InProgress {
            return Err(crate::error::AppError::Conflict(
                "decision resolution cannot enter in_progress without a lease; use todo and claim the Issue"
                    .to_owned(),
            ));
        }
        let resolution = resolution_input.into_resolution()?;
        let (issue_id, project_id, issue_number) = transaction
            .query_row(
                "SELECT request.issue_id, issue.project_id, issue.number
                 FROM decision_requests request
                 JOIN issues issue ON issue.id = request.issue_id
                 WHERE request.id = ?1",
                [request_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(crate::error::AppError::from)?;
        let issue_id = uuid::Uuid::parse_str(&issue_id)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        let project_id = uuid::Uuid::parse_str(&project_id)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        if resolution.target_state() == crate::domain::IssueState::Done {
            let another_open_request = transaction
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM decision_requests
                         WHERE issue_id = ?1 AND status = 'open' AND id <> ?2
                     )",
                    rusqlite::params![issue_id.to_string(), request_id.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(crate::error::AppError::from)?;
            if another_open_request {
                return Err(crate::error::AppError::Conflict(
                    "Issue has another unresolved human decision".to_owned(),
                ));
            }
        }

        let issue = Self::issue_in_transaction(&transaction, project_id, issue_number)?;
        if let Some(expected_revision) = expected_revision
            && issue.revision != expected_revision
        {
            return Err(crate::error::AppError::RevisionConflict {
                current_revision: issue.revision,
            });
        }
        let now = chrono::Utc::now();
        transaction
            .execute(
                "UPDATE issues
                 SET state = ?1, revision = revision + 1, updated_at = ?2
                 WHERE id = ?3 AND revision = ?4",
                rusqlite::params![
                    resolution.target_state().as_str(),
                    now.to_rfc3339(),
                    issue.id.to_string(),
                    issue.revision,
                ],
            )
            .map_err(crate::error::AppError::from)?;
        transaction
            .execute(
                "UPDATE decision_requests
                 SET status = 'resolved', answer = ?1, resolver_kind = ?2,
                     resolver_name = ?3, resolver_session_id = ?4, resolved_at = ?5
                 WHERE id = ?6 AND status = 'open'",
                rusqlite::params![
                    answer,
                    context.kind.as_str(),
                    context.initiator_name(),
                    context.session_id,
                    now.to_rfc3339(),
                    request_id.to_string(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let mut updated_issue = issue.clone();
        updated_issue.state = resolution.target_state();
        updated_issue.revision += 1;
        updated_issue.updated_at = now;
        let mut resolution_metadata =
            resolution.event_metadata(issue.state, updated_issue.revision);
        resolution_metadata
            .as_object_mut()
            .expect("decision resolution metadata is an object")
            .insert("request_id".to_owned(), request_id.to_string().into());
        let event_metadata = Self::event_metadata(resolution_metadata, context)?;
        Self::insert_domain_event(
            &transaction,
            &updated_issue,
            resolution.event_type(),
            &event_metadata,
            now,
        )?;
        let issue_id_string = updated_issue.id.to_string();
        let project_name = Self::project_name_in_transaction(&transaction, project_id)?;
        let audit_metadata = serde_json::json!({
            "request_id": request_id,
            "revision": updated_issue.revision,
        })
        .to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "decision_resolve",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &issue_id_string)),
                project: Some((project_id, &project_name)),
                revision: Some(updated_issue.revision),
                started_at: now,
                exit_code: 0,
                changed_fields: resolution.changed_fields(),
                metadata_json: &audit_metadata,
            },
        )?;
        request.status = "resolved".to_owned();
        request.answer = Some(answer.to_owned());
        request.resolver_kind = Some(context.kind);
        request.resolver_name = context.initiator_name().map(str::to_owned);
        request.resolver_session_id = context.session_id.clone();
        request.resolved_at = Some(now);
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &request)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(request)
    }

    pub fn add_dependency(
        &mut self,
        blocker: &crate::domain::IssueReference,
        blocked: &crate::domain::IssueReference,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueDependency, crate::error::AppError> {
        let idempotency = self.take_idempotency("issue_dependency_add")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(relation) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(relation);
        }
        let blocker_issue = Self::issue_reference_in_transaction(&transaction, blocker)?;
        let blocked_issue = Self::issue_reference_in_transaction(&transaction, blocked)?;
        if blocker_issue.id == blocked_issue.id {
            return Err(crate::error::AppError::Conflict(
                "an Issue cannot block itself".to_owned(),
            ));
        }
        let duplicate = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM issue_dependencies
                     WHERE blocker_issue_id = ?1 AND blocked_issue_id = ?2
                 )",
                rusqlite::params![blocker_issue.id.to_string(), blocked_issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if duplicate {
            return Err(crate::error::AppError::Conflict(
                "dependency already exists".to_owned(),
            ));
        }
        let creates_cycle = transaction
            .query_row(
                "WITH RECURSIVE reachable(issue_id) AS (
                     SELECT ?1
                     UNION
                     SELECT dependency.blocked_issue_id
                     FROM issue_dependencies dependency
                     JOIN reachable ON dependency.blocker_issue_id = reachable.issue_id
                 )
                 SELECT EXISTS(
                     SELECT 1 FROM reachable WHERE issue_id = ?2
                 )",
                rusqlite::params![blocked_issue.id.to_string(), blocker_issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if creates_cycle {
            return Err(crate::error::AppError::Conflict(
                "dependency would create a cycle".to_owned(),
            ));
        }

        let created_at = chrono::Utc::now();
        transaction
            .execute(
                "INSERT INTO issue_dependencies
                 (id, blocker_issue_id, blocked_issue_id, relation, created_at)
                 VALUES (?1, ?2, ?3, 'blocks', ?4)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    blocker_issue.id.to_string(),
                    blocked_issue.id.to_string(),
                    created_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let metadata = Self::event_metadata(
            serde_json::json!({
                "blocker": blocker.label(),
                "blocked": blocked.label(),
            }),
            context,
        )?;
        transaction
            .execute(
                "INSERT INTO domain_events
                 (id, sequence, project_id, issue_id, event_type, metadata_json, created_at)
                 VALUES (?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),
                         ?2, ?3, 'issue_dependency_added', ?4, ?5)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    blocked_issue.project_id.to_string(),
                    blocked_issue.id.to_string(),
                    metadata,
                    created_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let blocked_id = blocked_issue.id.to_string();
        let blocked_project =
            Self::project_name_in_transaction(&transaction, blocked_issue.project_id)?;
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_dependency_add",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &blocked_id)),
                project: Some((blocked_issue.project_id, &blocked_project)),
                revision: None,
                started_at: created_at,
                exit_code: 0,
                changed_fields: &["dependencies"],
                metadata_json: "{}",
            },
        )?;
        let relation = crate::domain::IssueDependency {
            blocker: blocker.label(),
            blocked: blocked.label(),
            relation: "blocks".to_owned(),
            created_at,
        };
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &relation)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(relation)
    }

    pub fn remove_dependency(
        &mut self,
        blocker: &crate::domain::IssueReference,
        blocked: &crate::domain::IssueReference,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueDependency, crate::error::AppError> {
        let idempotency = self.take_idempotency("issue_dependency_remove")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(relation) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(relation);
        }
        let blocker_issue = Self::issue_reference_in_transaction(&transaction, blocker)?;
        let blocked_issue = Self::issue_reference_in_transaction(&transaction, blocked)?;
        let created_at_text = transaction
            .query_row(
                "SELECT created_at FROM issue_dependencies
                 WHERE blocker_issue_id = ?1 AND blocked_issue_id = ?2",
                rusqlite::params![blocker_issue.id.to_string(), blocked_issue.id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("dependency not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })?;
        transaction
            .execute(
                "DELETE FROM issue_dependencies
                 WHERE blocker_issue_id = ?1 AND blocked_issue_id = ?2",
                rusqlite::params![blocker_issue.id.to_string(), blocked_issue.id.to_string()],
            )
            .map_err(crate::error::AppError::from)?;
        let created_at = Self::parse_timestamp(&created_at_text)?;
        let event_time = chrono::Utc::now();
        let metadata = Self::event_metadata(
            serde_json::json!({
                "blocker": blocker.label(),
                "blocked": blocked.label(),
            }),
            context,
        )?;
        transaction
            .execute(
                "INSERT INTO domain_events
                 (id, sequence, project_id, issue_id, event_type, metadata_json, created_at)
                 VALUES (?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),
                         ?2, ?3, 'issue_dependency_removed', ?4, ?5)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    blocked_issue.project_id.to_string(),
                    blocked_issue.id.to_string(),
                    metadata,
                    event_time.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let blocked_id = blocked_issue.id.to_string();
        let blocked_project =
            Self::project_name_in_transaction(&transaction, blocked_issue.project_id)?;
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_dependency_remove",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &blocked_id)),
                project: Some((blocked_issue.project_id, &blocked_project)),
                revision: None,
                started_at: event_time,
                exit_code: 0,
                changed_fields: &["dependencies"],
                metadata_json: "{}",
            },
        )?;
        let relation = crate::domain::IssueDependency {
            blocker: blocker.label(),
            blocked: blocked.label(),
            relation: "blocks".to_owned(),
            created_at,
        };
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &relation)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(relation)
    }

    pub fn list_dependencies(
        &self,
        reference: &crate::domain::IssueReference,
    ) -> Result<Vec<crate::domain::IssueDependency>, crate::error::AppError> {
        let issue = self.issue_reference(reference)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT blocker_project.name, blocker.number,
                        blocked_project.name, blocked.number, dependency.created_at
                 FROM issue_dependencies dependency
                 JOIN issues blocker ON blocker.id = dependency.blocker_issue_id
                 JOIN projects blocker_project ON blocker_project.id = blocker.project_id
                 JOIN issues blocked ON blocked.id = dependency.blocked_issue_id
                 JOIN projects blocked_project ON blocked_project.id = blocked.project_id
                 WHERE dependency.blocker_issue_id = ?1 OR dependency.blocked_issue_id = ?1
                 ORDER BY dependency.created_at ASC",
            )
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query([issue.id.to_string()])
            .map_err(crate::error::AppError::from)?;
        let mut dependencies = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            dependencies.push(Self::dependency_from_row(row)?);
        }
        Ok(dependencies)
    }

    pub fn set_parent(
        &mut self,
        child: &crate::domain::IssueReference,
        parent: &crate::domain::IssueReference,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueParent, crate::error::AppError> {
        let idempotency = self.take_idempotency("issue_parent_set")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(parent) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(parent);
        }
        let child_issue = Self::issue_reference_in_transaction(&transaction, child)?;
        let parent_issue = Self::issue_reference_in_transaction(&transaction, parent)?;
        if child_issue.id == parent_issue.id {
            return Err(crate::error::AppError::Conflict(
                "an Issue cannot be its own parent".to_owned(),
            ));
        }
        let child_has_parent = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM issue_parents WHERE child_issue_id = ?1)",
                [child_issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if child_has_parent {
            return Err(crate::error::AppError::Conflict(
                "Issue already has a parent".to_owned(),
            ));
        }
        let child_has_children = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM issue_parents WHERE parent_issue_id = ?1)",
                [child_issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if child_has_children {
            return Err(crate::error::AppError::Conflict(
                "parent nesting is limited to one level".to_owned(),
            ));
        }
        let parent_has_parent = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM issue_parents WHERE child_issue_id = ?1)",
                [parent_issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if parent_has_parent {
            return Err(crate::error::AppError::Conflict(
                "parent nesting is limited to one level".to_owned(),
            ));
        }
        let created_at = chrono::Utc::now();
        transaction
            .execute(
                "INSERT INTO issue_parents (child_issue_id, parent_issue_id, created_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    child_issue.id.to_string(),
                    parent_issue.id.to_string(),
                    created_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let metadata = Self::event_metadata(
            serde_json::json!({ "child": child.label(), "parent": parent.label() }),
            context,
        )?;
        transaction
            .execute(
                "INSERT INTO domain_events
                 (id, sequence, project_id, issue_id, event_type, metadata_json, created_at)
                 VALUES (?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),
                         ?2, ?3, 'issue_parent_set', ?4, ?5)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    child_issue.project_id.to_string(),
                    child_issue.id.to_string(),
                    metadata,
                    created_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let child_id = child_issue.id.to_string();
        let child_project =
            Self::project_name_in_transaction(&transaction, child_issue.project_id)?;
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_parent_set",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &child_id)),
                project: Some((child_issue.project_id, &child_project)),
                revision: None,
                started_at: created_at,
                exit_code: 0,
                changed_fields: &["parent"],
                metadata_json: "{}",
            },
        )?;
        let relation = crate::domain::IssueParent {
            child: child.label(),
            parent: parent.label(),
            created_at,
        };
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &relation)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(relation)
    }

    pub fn list_parent(
        &self,
        reference: &crate::domain::IssueReference,
    ) -> Result<Vec<crate::domain::IssueParent>, crate::error::AppError> {
        let issue = self.issue_reference(reference)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT child_project.name, child.number,
                        parent_project.name, parent.number, relation.created_at
                 FROM issue_parents relation
                 JOIN issues child ON child.id = relation.child_issue_id
                 JOIN projects child_project ON child_project.id = child.project_id
                 JOIN issues parent ON parent.id = relation.parent_issue_id
                 JOIN projects parent_project ON parent_project.id = parent.project_id
                 WHERE relation.child_issue_id = ?1",
            )
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query([issue.id.to_string()])
            .map_err(crate::error::AppError::from)?;
        let mut parents = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            parents.push(Self::parent_from_row(row)?);
        }
        Ok(parents)
    }

    pub fn claim_issue(
        &mut self,
        project: Option<&str>,
        number: Option<i64>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::ClaimedIssue, crate::error::AppError> {
        let idempotency = self.take_idempotency("issue_claim")?;
        let (agent, session_id) = Self::agent_session(context)?;
        if number.is_some_and(|number| number < 1) {
            return Err(crate::error::AppError::InvalidInput(
                "issue number must be positive".to_owned(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(claimed) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(claimed);
        }
        let issue = match number {
            Some(number) => {
                let project = project.ok_or_else(|| {
                    crate::error::AppError::InvalidInput(
                        "--project is required when claiming an Issue by number".to_owned(),
                    )
                })?;
                let reference = crate::domain::IssueReference {
                    project: project.to_owned(),
                    number,
                };
                let issue = Self::issue_reference_in_transaction(&transaction, &reference)?;
                Self::ensure_claimable(&transaction, &issue)?;
                issue
            }
            None => {
                let mut sql = String::from(
                    "SELECT i.id, i.project_id, i.number, i.title, i.body, i.state,
                            i.priority, i.assignee_kind, i.assignee_name, i.revision,
                            i.created_at, i.updated_at
                     FROM issues i
                     JOIN projects p ON p.id = i.project_id
                     WHERE i.state = 'todo'
                       AND NOT EXISTS (
                           SELECT 1 FROM issue_dependencies dependency
                           JOIN issues blocker ON blocker.id = dependency.blocker_issue_id
                           WHERE dependency.blocked_issue_id = i.id
                             AND blocker.state NOT IN ('done', 'cancelled')
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM decision_requests request
                           WHERE request.issue_id = i.id AND request.status = 'open'
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM issue_leases lease WHERE lease.issue_id = i.id
                       )",
                );
                let mut parameters = Vec::<rusqlite::types::Value>::new();
                if let Some(project) = project {
                    sql.push_str(" AND p.name = ?");
                    parameters.push(project.to_owned().into());
                }
                sql.push_str(
                    " ORDER BY CASE i.priority
                           WHEN 'critical' THEN 0 WHEN 'high' THEN 1
                           WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END,
                           i.created_at ASC, p.name ASC, i.number ASC
                       LIMIT 1",
                );
                let mut statement = transaction
                    .prepare(&sql)
                    .map_err(crate::error::AppError::from)?;
                statement
                    .query_row(
                        rusqlite::params_from_iter(parameters.iter()),
                        Self::issue_from_row,
                    )
                    .map_err(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => {
                            crate::error::AppError::NotFound("no claimable Issue found".to_owned())
                        }
                        error => crate::error::AppError::from(error),
                    })?
            }
        };

        let now = chrono::Utc::now();
        let expires_at = now + chrono::Duration::minutes(LEASE_TTL_MINUTES);
        let changed = transaction
            .execute(
                "UPDATE issues
                 SET state = 'in_progress', assignee_kind = 'agent', assignee_name = ?1,
                     revision = revision + 1, updated_at = ?2
                 WHERE id = ?3 AND revision = ?4 AND state = 'todo'",
                rusqlite::params![
                    agent,
                    now.to_rfc3339(),
                    issue.id.to_string(),
                    issue.revision
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
        transaction
            .execute(
                "INSERT INTO issue_leases
                 (issue_id, agent, session_id, claimed_at, heartbeat_at, expires_at, lease_revision)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5, 1)",
                rusqlite::params![
                    issue.id.to_string(),
                    agent,
                    session_id,
                    now.to_rfc3339(),
                    expires_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let mut updated_issue = issue.clone();
        updated_issue.state = crate::domain::IssueState::InProgress;
        updated_issue.assignee_kind = Some(crate::domain::AssigneeKind::Agent);
        updated_issue.assignee_name = Some(agent.to_owned());
        updated_issue.revision += 1;
        updated_issue.updated_at = now;
        let metadata = Self::event_metadata(
            serde_json::json!({
                "revision": updated_issue.revision,
                "lease_expires_at": expires_at.to_rfc3339(),
            }),
            context,
        )?;
        Self::insert_domain_event(
            &transaction,
            &updated_issue,
            "issue_claimed",
            &metadata,
            now,
        )?;
        let issue_id = updated_issue.id.to_string();
        let project_name =
            Self::project_name_in_transaction(&transaction, updated_issue.project_id)?;
        let audit_metadata = serde_json::json!({
            "revision": updated_issue.revision,
            "expires_at": expires_at.to_rfc3339(),
        })
        .to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_claim",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &issue_id)),
                project: Some((updated_issue.project_id, &project_name)),
                revision: Some(updated_issue.revision),
                started_at: now,
                exit_code: 0,
                changed_fields: &["state", "assignee_kind", "assignee_name"],
                metadata_json: &audit_metadata,
            },
        )?;
        let lease = crate::domain::IssueLease {
            agent: agent.to_owned(),
            session_id: session_id.to_owned(),
            claimed_at: now,
            heartbeat_at: now,
            expires_at,
            lease_revision: 1,
            stale: false,
        };
        let claimed = crate::domain::ClaimedIssue {
            issue: updated_issue,
            lease,
        };
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &claimed)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(claimed)
    }

    pub fn heartbeat_issue(
        &mut self,
        issue: &crate::domain::Issue,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::ClaimedIssue, crate::error::AppError> {
        let idempotency = self.take_idempotency("issue_heartbeat")?;
        let (agent, session_id) = Self::agent_session(context)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(claimed) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(claimed);
        }
        let current = Self::lease_in_transaction(&transaction, issue.id)?;
        if current.agent != agent || current.session_id != session_id {
            return Err(crate::error::AppError::Conflict(
                "lease is owned by another agent session".to_owned(),
            ));
        }
        let now = chrono::Utc::now();
        if current.expires_at <= now {
            return Err(crate::error::AppError::Conflict(
                "lease has expired; use takeover with a reason".to_owned(),
            ));
        }
        let expires_at = now + chrono::Duration::minutes(LEASE_TTL_MINUTES);
        transaction
            .execute(
                "UPDATE issue_leases
                 SET heartbeat_at = ?1, expires_at = ?2, lease_revision = lease_revision + 1
                 WHERE issue_id = ?3 AND agent = ?4 AND session_id = ?5",
                rusqlite::params![
                    now.to_rfc3339(),
                    expires_at.to_rfc3339(),
                    issue.id.to_string(),
                    agent,
                    session_id,
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let lease = Self::lease_in_transaction(&transaction, issue.id)?;
        let issue = Self::issue_in_transaction(&transaction, issue.project_id, issue.number)?;
        let issue_id = issue.id.to_string();
        let project_name = Self::project_name_in_transaction(&transaction, issue.project_id)?;
        let audit_metadata = serde_json::json!({
            "expires_at": expires_at.to_rfc3339(),
            "lease_revision": lease.lease_revision,
        })
        .to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_heartbeat",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &issue_id)),
                project: Some((issue.project_id, &project_name)),
                revision: Some(issue.revision),
                started_at: now,
                exit_code: 0,
                changed_fields: &[],
                metadata_json: &audit_metadata,
            },
        )?;
        let claimed = crate::domain::ClaimedIssue { issue, lease };
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &claimed)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(claimed)
    }

    pub fn takeover_issue(
        &mut self,
        issue: &crate::domain::Issue,
        reason: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::ClaimedIssue, crate::error::AppError> {
        let idempotency = self.take_idempotency("issue_takeover")?;
        let (agent, session_id) = Self::agent_session(context)?;
        if reason.trim().is_empty() {
            return Err(crate::error::AppError::InvalidInput(
                "takeover reason must not be empty".to_owned(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(claimed) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(claimed);
        }
        let current = Self::lease_in_transaction(&transaction, issue.id)?;
        let now = chrono::Utc::now();
        if current.expires_at > now {
            return Err(crate::error::AppError::Conflict(
                "lease is still active".to_owned(),
            ));
        }
        let expires_at = now + chrono::Duration::minutes(LEASE_TTL_MINUTES);
        let updated = transaction
            .execute(
                "UPDATE issues
                 SET state = 'in_progress', assignee_kind = 'agent', assignee_name = ?1,
                     revision = revision + 1, updated_at = ?2
                 WHERE id = ?3 AND revision = ?4",
                rusqlite::params![
                    agent,
                    now.to_rfc3339(),
                    issue.id.to_string(),
                    issue.revision
                ],
            )
            .map_err(crate::error::AppError::from)?;
        if updated == 0 {
            let current_revision = transaction
                .query_row(
                    "SELECT revision FROM issues WHERE id = ?1",
                    [issue.id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(crate::error::AppError::from)?;
            return Err(crate::error::AppError::RevisionConflict { current_revision });
        }
        transaction
            .execute(
                "UPDATE issue_leases
                 SET agent = ?1, session_id = ?2, claimed_at = ?3,
                     heartbeat_at = ?3, expires_at = ?4, lease_revision = lease_revision + 1
                 WHERE issue_id = ?5",
                rusqlite::params![
                    agent,
                    session_id,
                    now.to_rfc3339(),
                    expires_at.to_rfc3339(),
                    issue.id.to_string(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        let mut updated_issue = issue.clone();
        updated_issue.state = crate::domain::IssueState::InProgress;
        updated_issue.assignee_kind = Some(crate::domain::AssigneeKind::Agent);
        updated_issue.assignee_name = Some(agent.to_owned());
        updated_issue.revision += 1;
        updated_issue.updated_at = now;
        let metadata = Self::event_metadata(
            serde_json::json!({
                "revision": updated_issue.revision,
                "reason": reason,
                "lease_expires_at": expires_at.to_rfc3339(),
            }),
            context,
        )?;
        Self::insert_domain_event(
            &transaction,
            &updated_issue,
            "issue_taken_over",
            &metadata,
            now,
        )?;
        let issue_id = updated_issue.id.to_string();
        let project_name =
            Self::project_name_in_transaction(&transaction, updated_issue.project_id)?;
        let audit_metadata = serde_json::json!({
            "reason": reason,
            "expires_at": expires_at.to_rfc3339(),
        })
        .to_string();
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "issue_takeover",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
                success: true,
                context,
                target: Some(("issue", &issue_id)),
                project: Some((updated_issue.project_id, &project_name)),
                revision: Some(updated_issue.revision),
                started_at: now,
                exit_code: 0,
                changed_fields: &["state", "assignee_kind", "assignee_name"],
                metadata_json: &audit_metadata,
            },
        )?;
        let lease = Self::lease_in_transaction(&transaction, updated_issue.id)?;
        let claimed = crate::domain::ClaimedIssue {
            issue: updated_issue,
            lease,
        };
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &claimed)?;
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(claimed)
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
                CASE i.state
                    WHEN 'blocked' THEN 0
                    WHEN 'in_progress' THEN 1
                    WHEN 'todo' THEN 2
                    WHEN 'done' THEN 3
                    WHEN 'cancelled' THEN 4
                    ELSE 5
                END ASC,
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

    pub fn list_stale_issues(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<crate::domain::IssueListItem>, crate::error::AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT p.name, i.id, i.project_id, i.number, i.title, i.body, i.state,
                        i.priority, i.assignee_kind, i.assignee_name, i.revision,
                        i.created_at, i.updated_at
                 FROM issues i
                 JOIN projects p ON p.id = i.project_id
                 JOIN issue_leases lease ON lease.issue_id = i.id
                 WHERE i.state = 'in_progress' AND lease.expires_at <= ?1
                 ORDER BY i.updated_at ASC, p.name ASC, i.number ASC",
            )
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query([now.to_rfc3339()])
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

    pub fn list_attention_issues(
        &self,
    ) -> Result<Vec<crate::domain::IssueListItem>, crate::error::AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT p.name, i.id, i.project_id, i.number, i.title, i.body, i.state,
                        i.priority, i.assignee_kind, i.assignee_name, i.revision,
                        i.created_at, i.updated_at
                 FROM issues i
                 JOIN projects p ON p.id = i.project_id
                 WHERE EXISTS (
                     SELECT 1 FROM decision_requests request
                     WHERE request.issue_id = i.id AND request.status = 'open'
                 )
                 ORDER BY i.updated_at ASC, p.name ASC, i.number ASC",
            )
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement.query([]).map_err(crate::error::AppError::from)?;
        let mut issues = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            issues.push(crate::domain::IssueListItem {
                project: row.get(0).map_err(crate::error::AppError::from)?,
                issue: Self::issue_from_row_at(row, 1).map_err(crate::error::AppError::from)?,
            });
        }
        Ok(issues)
    }

    pub fn list_events(
        &self,
        after: i64,
        limit: usize,
        include_issue: bool,
    ) -> Result<crate::domain::EventPage, crate::error::AppError> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(crate::error::AppError::from)?;
        let mut statement = transaction
            .prepare(
                "SELECT sequence, event_type, project_id, issue_id, metadata_json, created_at
                 FROM domain_events
                 WHERE sequence > ?1
                 ORDER BY sequence ASC
                 LIMIT ?2",
            )
            .map_err(crate::error::AppError::from)?;
        let mut rows = statement
            .query(rusqlite::params![after, i64::try_from(limit + 1).unwrap()])
            .map_err(crate::error::AppError::from)?;
        let mut raw_events = Vec::new();
        while let Some(row) = rows.next().map_err(crate::error::AppError::from)? {
            raw_events.push((
                row.get::<_, i64>(0).map_err(crate::error::AppError::from)?,
                row.get::<_, String>(1)
                    .map_err(crate::error::AppError::from)?,
                row.get::<_, Option<String>>(2)
                    .map_err(crate::error::AppError::from)?,
                row.get::<_, Option<String>>(3)
                    .map_err(crate::error::AppError::from)?,
                row.get::<_, String>(4)
                    .map_err(crate::error::AppError::from)?,
                row.get::<_, String>(5)
                    .map_err(crate::error::AppError::from)?,
            ));
        }
        drop(rows);
        drop(statement);

        let has_more = raw_events.len() > limit;
        if has_more {
            raw_events.pop();
        }
        let next_cursor = raw_events.last().map_or(after, |event| event.0);
        let mut events = Vec::with_capacity(raw_events.len());
        for (sequence, event_type, project_id, issue_id, metadata_json, created_at) in raw_events {
            let metadata: serde_json::Value = serde_json::from_str(&metadata_json)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
            let project_id = project_id
                .map(|value| {
                    uuid::Uuid::parse_str(&value)
                        .map_err(|error| crate::error::AppError::Internal(error.to_string()))
                })
                .transpose()?;
            let issue_id = issue_id
                .map(|value| {
                    uuid::Uuid::parse_str(&value)
                        .map_err(|error| crate::error::AppError::Internal(error.to_string()))
                })
                .transpose()?;
            let issue = if include_issue {
                match issue_id {
                    Some(issue_id) => match transaction.query_row(
                        "SELECT id, project_id, number, title, body, state,
                                priority, assignee_kind, assignee_name, revision,
                                created_at, updated_at
                         FROM issues WHERE id = ?1",
                        [issue_id.to_string()],
                        Self::issue_from_row,
                    ) {
                        Ok(issue) => Some(issue),
                        Err(rusqlite::Error::QueryReturnedNoRows) => None,
                        Err(error) => return Err(crate::error::AppError::from(error)),
                    },
                    None => None,
                }
            } else {
                None
            };
            let created_at = Self::parse_timestamp(&created_at)?;
            events.push(crate::domain::EventRecord {
                sequence,
                event_type: event_type.clone(),
                project_id,
                issue_id,
                changed_fields: Self::event_changed_fields(&event_type, &metadata),
                revision: metadata.get("revision").and_then(serde_json::Value::as_i64),
                created_at,
                issue,
            });
        }
        transaction.commit().map_err(crate::error::AppError::from)?;
        Ok(crate::domain::EventPage {
            next_cursor,
            has_more,
            events,
        })
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
                    target_type, target_id, revision, idempotency_key, changed_fields_json
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
        if !self.audit_enabled {
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation,
                idempotency_key: None,
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

    fn check_idempotency<T>(
        transaction: &rusqlite::Transaction<'_>,
        request: Option<&IdempotencyRequest>,
    ) -> Result<Option<T>, crate::error::AppError>
    where
        T: serde::de::DeserializeOwned,
    {
        let Some(request) = request else {
            return Ok(None);
        };
        use rusqlite::OptionalExtension as _;
        let record = transaction
            .query_row(
                "SELECT operation, request_hash, response_json
                 FROM idempotency_records WHERE idempotency_key = ?1",
                [&request.key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(crate::error::AppError::from)?;
        let Some((operation, request_hash, response_json)) = record else {
            return Ok(None);
        };
        if operation != request.operation || request_hash != request.request_hash {
            return Err(crate::error::AppError::IdempotencyConflict);
        }
        serde_json::from_str(&response_json)
            .map(Some)
            .map_err(|error| {
                crate::error::AppError::Internal(format!(
                    "stored idempotency response is invalid: {error}"
                ))
            })
    }

    pub(crate) fn lookup_idempotency<T>(
        &self,
        request: Option<&IdempotencyRequest>,
    ) -> Result<Option<T>, crate::error::AppError>
    where
        T: serde::de::DeserializeOwned,
    {
        let Some(request) = request else {
            return Ok(None);
        };
        use rusqlite::OptionalExtension as _;
        let record = self
            .connection
            .query_row(
                "SELECT operation, request_hash, response_json
                 FROM idempotency_records WHERE idempotency_key = ?1",
                [&request.key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(crate::error::AppError::from)?;
        let Some((operation, request_hash, response_json)) = record else {
            return Ok(None);
        };
        if operation != request.operation || request_hash != request.request_hash {
            return Err(crate::error::AppError::IdempotencyConflict);
        }
        serde_json::from_str(&response_json)
            .map(Some)
            .map_err(|error| {
                crate::error::AppError::Internal(format!(
                    "stored idempotency response is invalid: {error}"
                ))
            })
    }

    fn remember_idempotency<T>(
        transaction: &rusqlite::Transaction<'_>,
        request: Option<&IdempotencyRequest>,
        result: &T,
    ) -> Result<(), crate::error::AppError>
    where
        T: serde::Serialize,
    {
        let Some(request) = request else {
            return Ok(());
        };
        let response_json = serde_json::to_string(result)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO idempotency_records
                 (idempotency_key, operation, request_hash, response_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    request.key,
                    request.operation,
                    request.request_hash,
                    response_json,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        Ok(())
    }

    pub fn record_failed_operation(
        &mut self,
        operation: &str,
        context: &crate::domain::ExecutionContext,
        error: &crate::error::AppError,
        subject: &AuditSubject,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), crate::error::AppError> {
        self.record_failed_operation_with_idempotency(
            operation, context, error, subject, started_at, None,
        )
    }

    pub(crate) fn record_failed_operation_with_idempotency(
        &mut self,
        operation: &str,
        context: &crate::domain::ExecutionContext,
        error: &crate::error::AppError,
        subject: &AuditSubject,
        started_at: chrono::DateTime<chrono::Utc>,
        idempotency_key: Option<&str>,
    ) -> Result<(), crate::error::AppError> {
        // A failure audit uses the same writer lock, so it cannot be persisted while
        // the original operation is reporting that lock as busy. Preserve the
        // actionable busy error instead of waiting again and replacing it.
        if !self.audit_enabled || matches!(error, crate::error::AppError::DatabaseBusy(_)) {
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
                idempotency_key,
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
                    target_type, target_id, revision, idempotency_key, changed_fields_json,
                    metadata_json
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
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
                    event.idempotency_key,
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
        let idempotency_key: Option<String> = row.get(14).map_err(crate::error::AppError::from)?;
        let changed_fields = Self::audit_changed_fields_from_row(&operation, row, 15)?;
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
            idempotency_key,
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
                | "issue_batch"
                | "issue_edit"
                | "issue_comment"
                | "issue_history"
                | "issue_dependency_add"
                | "issue_dependency_remove"
                | "issue_dependency_list"
                | "issue_parent_set"
                | "issue_parent_list"
                | "issue_claim"
                | "issue_heartbeat"
                | "issue_takeover"
                | "decision_request"
                | "decision_list"
                | "decision_resolve"
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
            "issue_create"
            | "issue_show"
            | "issue_edit"
            | "issue_comment"
            | "issue_history"
            | "issue_dependency_add"
            | "issue_dependency_remove"
            | "issue_dependency_list"
            | "issue_parent_set"
            | "issue_parent_list"
            | "issue_claim"
            | "issue_heartbeat"
            | "issue_takeover"
            | "decision_request"
            | "decision_list"
            | "decision_resolve"
            | "issue_start"
            | "issue_block"
            | "issue_resume"
            | "issue_complete"
            | "issue_cancel"
            | "issue_reopen" => Some("issue"),
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
                | "issue_dependency_add"
                | "issue_dependency_remove"
                | "issue_dependency_list"
                | "issue_parent_set"
                | "issue_parent_list"
                | "issue_claim"
                | "issue_heartbeat"
                | "issue_takeover"
                | "decision_request"
                | "decision_list"
                | "decision_resolve"
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
            "issue_dependency_add" | "issue_dependency_remove" => &["dependencies"],
            "issue_dependency_list" => &[],
            "issue_parent_set" => &["parent"],
            "issue_parent_list" => &[],
            "issue_claim" => &["state", "assignee_kind", "assignee_name"],
            "issue_heartbeat" => &[],
            "issue_takeover" => &["state", "assignee_kind", "assignee_name"],
            "decision_request" => &["state", "decision_request"],
            "decision_list" => &[],
            "decision_resolve" => &[
                "decision",
                "state",
                "reason",
                "wait_kind",
                "summary",
                "verification",
            ],
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
        let idempotency = self.take_idempotency("issue_create")?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        if let Some(issue) = Self::check_idempotency(&transaction, idempotency.as_ref())? {
            return Ok(issue);
        }
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
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
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
        Self::remember_idempotency(&transaction, idempotency.as_ref(), &issue)?;
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

    fn issue_reference(
        &self,
        reference: &crate::domain::IssueReference,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let project_id = self.project_id(&reference.project)?;
        self.connection
            .query_row(
                "SELECT id, project_id, number, title, body, state, priority, assignee_kind,
                        assignee_name, revision, created_at, updated_at
                 FROM issues WHERE project_id = ?1 AND number = ?2",
                rusqlite::params![project_id.to_string(), reference.number],
                Self::issue_from_row,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("issue not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })
    }

    fn issue_reference_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        reference: &crate::domain::IssueReference,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let project_id = Self::project_id_in_transaction(transaction, &reference.project)?;
        Self::issue_in_transaction(transaction, project_id, reference.number)
    }

    fn agent_session(
        context: &crate::domain::ExecutionContext,
    ) -> Result<(&str, &str), crate::error::AppError> {
        if context.kind != crate::domain::InitiatorKind::Agent {
            return Err(crate::error::AppError::InvalidInput(
                "agent execution context is required".to_owned(),
            ));
        }
        let agent = context.agent.as_deref().ok_or_else(|| {
            crate::error::AppError::InvalidInput("BETTR_AGENT is required".to_owned())
        })?;
        let session_id = context.session_id.as_deref().ok_or_else(|| {
            crate::error::AppError::InvalidInput("BETTR_SESSION_ID is required".to_owned())
        })?;
        Ok((agent, session_id))
    }

    fn ensure_claimable(
        transaction: &rusqlite::Transaction<'_>,
        issue: &crate::domain::Issue,
    ) -> Result<(), crate::error::AppError> {
        if issue.state != crate::domain::IssueState::Todo {
            return Err(crate::error::AppError::Conflict(
                "Issue is not claimable in its current state".to_owned(),
            ));
        }
        let blocked = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM issue_dependencies dependency
                     JOIN issues blocker ON blocker.id = dependency.blocker_issue_id
                     WHERE dependency.blocked_issue_id = ?1
                       AND blocker.state NOT IN ('done', 'cancelled')
                 )",
                [issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if blocked {
            return Err(crate::error::AppError::Conflict(
                "Issue has unresolved blocking dependencies".to_owned(),
            ));
        }
        let attention_required = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM decision_requests
                     WHERE issue_id = ?1 AND status = 'open'
                 )",
                [issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if attention_required {
            return Err(crate::error::AppError::Conflict(
                "Issue has an unresolved human decision".to_owned(),
            ));
        }
        let leased = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM issue_leases WHERE issue_id = ?1)",
                [issue.id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(crate::error::AppError::from)?;
        if leased {
            return Err(crate::error::AppError::Conflict(
                "Issue already has a lease".to_owned(),
            ));
        }
        Ok(())
    }

    fn insert_domain_event(
        transaction: &rusqlite::Transaction<'_>,
        issue: &crate::domain::Issue,
        event_type: &str,
        metadata: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), crate::error::AppError> {
        transaction
            .execute(
                "INSERT INTO domain_events
                 (id, sequence, project_id, issue_id, event_type, metadata_json, created_at)
                 VALUES (?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),
                         ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    issue.project_id.to_string(),
                    issue.id.to_string(),
                    event_type,
                    metadata,
                    created_at.to_rfc3339(),
                ],
            )
            .map_err(crate::error::AppError::from)?;
        Ok(())
    }

    fn lease_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        issue_id: uuid::Uuid,
    ) -> Result<crate::domain::IssueLease, crate::error::AppError> {
        let (agent, session_id, claimed_at, heartbeat_at, expires_at, lease_revision) = transaction
            .query_row(
                "SELECT agent, session_id, claimed_at, heartbeat_at, expires_at, lease_revision
                     FROM issue_leases WHERE issue_id = ?1",
                [issue_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::Conflict("lease not found".to_owned())
                }
                error => crate::error::AppError::from(error),
            })?;
        let expires_at = Self::parse_timestamp(&expires_at)?;
        Ok(crate::domain::IssueLease {
            agent,
            session_id,
            claimed_at: Self::parse_timestamp(&claimed_at)?,
            heartbeat_at: Self::parse_timestamp(&heartbeat_at)?,
            stale: expires_at <= chrono::Utc::now(),
            expires_at,
            lease_revision,
        })
    }

    fn parse_timestamp(
        value: &str,
    ) -> Result<chrono::DateTime<chrono::Utc>, crate::error::AppError> {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))
    }

    fn dependency_from_row(
        row: &rusqlite::Row<'_>,
    ) -> Result<crate::domain::IssueDependency, crate::error::AppError> {
        let blocker_project: String = row.get(0).map_err(crate::error::AppError::from)?;
        let blocker_number: i64 = row.get(1).map_err(crate::error::AppError::from)?;
        let blocked_project: String = row.get(2).map_err(crate::error::AppError::from)?;
        let blocked_number: i64 = row.get(3).map_err(crate::error::AppError::from)?;
        let created_at: String = row.get(4).map_err(crate::error::AppError::from)?;
        Ok(crate::domain::IssueDependency {
            blocker: format!("{blocker_project}#{blocker_number}"),
            blocked: format!("{blocked_project}#{blocked_number}"),
            relation: "blocks".to_owned(),
            created_at: Self::parse_timestamp(&created_at)?,
        })
    }

    fn parent_from_row(
        row: &rusqlite::Row<'_>,
    ) -> Result<crate::domain::IssueParent, crate::error::AppError> {
        let child_project: String = row.get(0).map_err(crate::error::AppError::from)?;
        let child_number: i64 = row.get(1).map_err(crate::error::AppError::from)?;
        let parent_project: String = row.get(2).map_err(crate::error::AppError::from)?;
        let parent_number: i64 = row.get(3).map_err(crate::error::AppError::from)?;
        let created_at: String = row.get(4).map_err(crate::error::AppError::from)?;
        Ok(crate::domain::IssueParent {
            child: format!("{child_project}#{child_number}"),
            parent: format!("{parent_project}#{parent_number}"),
            created_at: Self::parse_timestamp(&created_at)?,
        })
    }

    fn event_changed_fields(event_type: &str, metadata: &serde_json::Value) -> Vec<String> {
        let fields: Vec<&str> = match event_type {
            "project_created" => vec!["name"],
            "issue_created" => vec!["title", "body", "state", "priority"],
            "issue_updated" => metadata
                .get("changes")
                .and_then(serde_json::Value::as_object)
                .map(|changes| {
                    changes
                        .keys()
                        .filter_map(|field| match field.as_str() {
                            "title" | "body" | "priority" | "assignee_kind" | "assignee_name" => {
                                Some(field.as_str())
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            "comment_added" => vec!["comment", "updated_at"],
            "issue_started" | "issue_resumed" => vec!["state"],
            "issue_blocked" => vec!["state", "reason", "wait_kind"],
            "issue_completed" => vec!["state", "summary", "verification"],
            "issue_cancelled" | "issue_reopened" => vec!["state", "reason"],
            "issue_dependency_added" | "issue_dependency_removed" => vec!["dependencies"],
            "issue_parent_set" => vec!["parent"],
            "issue_claimed" => vec!["state", "assignee_kind", "assignee_name"],
            "issue_taken_over" => vec!["state", "assignee_kind", "assignee_name"],
            "decision_requested" => vec!["state", "decision_request"],
            "decision_resolved" => vec!["decision", "state"],
            _ => Vec::new(),
        };
        fields.into_iter().map(str::to_owned).collect()
    }

    fn decision_from_row(
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<crate::domain::DecisionRequest> {
        let parse_error = |error: Box<dyn std::error::Error + Send + Sync>| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error)
        };
        let id: String = row.get(0)?;
        let project: String = row.get(1)?;
        let number: i64 = row.get(2)?;
        let requester_kind: Option<String> = row.get(5)?;
        let resolver_kind: Option<String> = row.get(10)?;
        let parse_kind = |value: Option<String>| {
            value
                .map(|value| match value.as_str() {
                    "agent" => Ok(crate::domain::InitiatorKind::Agent),
                    "human" => Ok(crate::domain::InitiatorKind::Human),
                    "system" => Ok(crate::domain::InitiatorKind::System),
                    _ => Err(crate::error::AppError::Internal(format!(
                        "invalid decision initiator kind: {value}"
                    ))),
                })
                .transpose()
        };
        let created_at: String = row.get(13)?;
        let resolved_at: Option<String> = row.get(14)?;
        let options_json: String = row.get(16)?;
        let options =
            serde_json::from_str(&options_json).map_err(|error| parse_error(Box::new(error)))?;
        let id = uuid::Uuid::parse_str(&id).map_err(|error| parse_error(Box::new(error)))?;
        let requester_kind =
            parse_kind(requester_kind).map_err(|error| parse_error(Box::new(error)))?;
        let resolver_kind =
            parse_kind(resolver_kind).map_err(|error| parse_error(Box::new(error)))?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
            .map_err(|error| parse_error(Box::new(error)))?
            .with_timezone(&chrono::Utc);
        let resolved_at = resolved_at
            .map(|value| {
                chrono::DateTime::parse_from_rfc3339(&value)
                    .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
                    .map_err(|error| parse_error(Box::new(error)))
            })
            .transpose()?;
        Ok(crate::domain::DecisionRequest {
            id,
            issue: format!("{project}#{number}"),
            question: row.get(3)?,
            background: row.get(4)?,
            blocker: row.get(15)?,
            options,
            recommendation: row.get(17)?,
            resume_condition: row.get(18)?,
            requester_kind,
            requester_name: row.get(6)?,
            requester_session_id: row.get(7)?,
            status: row.get(8)?,
            answer: row.get(9)?,
            resolver_kind,
            resolver_name: row.get(11)?,
            resolver_session_id: row.get(12)?,
            created_at,
            resolved_at,
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
        let identity = Self::connection_identity(&connection)?;
        if !identity.is_bettr_application() {
            return Err(crate::error::AppError::DatabaseNotInitialized);
        }
        if !crate::store::migrations::is_supported_version(identity.user_version) {
            return Err(crate::error::AppError::UnsupportedDatabaseSchemaVersion {
                found_version: identity.user_version,
                current_version: crate::store::migrations::LATEST_SCHEMA_VERSION,
            });
        }

        let mut database = Self::configure_connection(connection)?;
        database.apply_pending_migrations(identity.user_version)?;
        Ok(database)
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

        Ok(Self {
            connection,
            pending_idempotency: None,
            audit_enabled: true,
        })
    }

    fn is_initialized_database(path: &std::path::Path) -> Result<bool, crate::error::AppError> {
        Ok(read_sqlite_header_identity(path)?.is_supported_bettr())
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
        idempotency: Option<&IdempotencyRequest>,
    ) -> Result<(), crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        transaction
            .execute_batch(include_str!("schema.sql"))
            .map_err(crate::error::AppError::from)?;
        let applied_at = chrono::Utc::now().to_rfc3339();
        for (version, name) in [
            (
                crate::store::migrations::BASE_SCHEMA_VERSION,
                "phase1_baseline",
            ),
            (2, "schema_migrations"),
            (3, "phase_two_coordination"),
            (4, "idempotency_and_audit"),
            (5, "blocked_decision_context"),
        ] {
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version, name, applied_at)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![version, name, &applied_at],
                )
                .map_err(crate::error::AppError::from)?;
        }
        Self::insert_audit_event(
            &transaction,
            AuditInsert {
                operation: "init",
                idempotency_key: idempotency.as_ref().map(|request| request.key.as_str()),
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
        let result = serde_json::json!({ "initialized": true });
        Self::remember_idempotency(&transaction, idempotency, &result)?;
        transaction.commit().map_err(crate::error::AppError::from)
    }

    fn apply_pending_migrations(
        &mut self,
        current_version: u32,
    ) -> Result<(), crate::error::AppError> {
        if current_version >= crate::store::migrations::LATEST_SCHEMA_VERSION {
            return Ok(());
        }

        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::error::AppError::from)?;
        let applied = crate::store::migrations::apply_pending(
            &transaction,
            crate::store::migrations::migrations(),
        )
        .map_err(crate::error::AppError::from)?;
        let context = crate::domain::ExecutionContext {
            kind: crate::domain::InitiatorKind::System,
            agent: None,
            session_id: None,
            operator: None,
        };
        let started_at = chrono::Utc::now();
        let mut from_version = current_version;
        for migration in applied {
            let metadata_json = serde_json::json!({
                "from_version": from_version,
                "to_version": migration.version,
                "migration": migration.name,
            })
            .to_string();
            Self::insert_audit_event(
                &transaction,
                AuditInsert {
                    operation: "schema_migrate",
                    idempotency_key: None,
                    success: true,
                    context: &context,
                    target: None,
                    project: None,
                    revision: None,
                    started_at,
                    exit_code: 0,
                    changed_fields: &[],
                    metadata_json: &metadata_json,
                },
            )?;
            from_version = migration.version;
        }
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

    fn decision_input(question: &str, background: &str) -> crate::domain::DecisionRequestInput {
        crate::domain::DecisionRequestInput {
            blocker: "The decision blocks implementation.".to_owned(),
            question: question.to_owned(),
            options: vec!["Use option A".to_owned(), "Use option B".to_owned()],
            recommendation: "Use option A".to_owned(),
            resume_condition: "The selected option is implemented and verified.".to_owned(),
            background: background.to_owned(),
        }
    }

    fn human_context() -> crate::domain::ExecutionContext {
        crate::domain::ExecutionContext {
            kind: crate::domain::InitiatorKind::Human,
            agent: None,
            session_id: None,
            operator: Some("reviewer".to_owned()),
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

    #[test]
    fn list_decisions_returns_requests_in_creation_order() {
        let (_directory, mut database) = initialized_database();
        database.create_project("bettr", &context()).unwrap();
        database
            .create_issue(
                "bettr",
                &crate::domain::NewIssue {
                    title: "decision list".to_owned(),
                    body: None,
                    priority: None,
                },
                &context(),
            )
            .unwrap();
        let parser_input = decision_input(
            "Choose the parser",
            "The parser has two compatible implementations.",
        );
        database
            .request_decision("bettr", 1, &parser_input, &context())
            .unwrap();
        let rollout_input =
            decision_input("Choose the rollout", "The rollout has two safe windows.");
        database
            .request_decision("bettr", 1, &rollout_input, &context())
            .unwrap();

        let decisions = database.list_decisions("bettr", 1).unwrap();

        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].question, "Choose the parser");
        assert_eq!(decisions[1].question, "Choose the rollout");
        assert_eq!(decisions[0].status, "open");
        assert_eq!(decisions[1].status, "open");
    }

    #[test]
    fn decision_resolution_rejects_a_stale_expected_revision() {
        let (_directory, mut database) = initialized_database();
        database.create_project("bettr", &context()).unwrap();
        let issue = database
            .create_issue(
                "bettr",
                &crate::domain::NewIssue {
                    title: "stale decision".to_owned(),
                    body: None,
                    priority: None,
                },
                &context(),
            )
            .unwrap();
        let input = decision_input(
            "Choose the parser",
            "The parser has two compatible implementations.",
        );
        let request = database
            .request_decision("bettr", 1, &input, &context())
            .unwrap();
        let result = database.resolve_decision(
            request.id,
            "Use option A",
            Some(1),
            crate::domain::DecisionResolutionInput::new(
                crate::domain::IssueState::Todo,
                None,
                None,
                None,
                None,
            ),
            &human_context(),
        );

        assert!(matches!(
            result,
            Err(crate::error::AppError::RevisionConflict {
                current_revision: 2
            })
        ));
        let (state, revision): (String, i64) = database
            .connection
            .query_row(
                "SELECT state, revision FROM issues WHERE id = ?1",
                [issue.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "blocked");
        assert_eq!(revision, 2);
        assert_eq!(
            database
                .connection
                .query_row(
                    "SELECT status FROM decision_requests WHERE id = ?1",
                    [request.id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "open"
        );
        assert_eq!(
            database
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM domain_events WHERE issue_id = ?1",
                    [issue.id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
    }
}
