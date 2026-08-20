#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    Argument,
    Environment,
    DirectoryConfig,
    UserConfig,
    Default,
}

impl ContextSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Argument => "argument",
            Self::Environment => "environment",
            Self::DirectoryConfig => "directory_config",
            Self::UserConfig => "user_config",
            Self::Default => "default",
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ResolvedValue<T> {
    pub value: T,
    pub source: ContextSource,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ResolvedContext {
    pub project: ResolvedValue<Option<String>>,
    pub database: ResolvedValue<std::path::PathBuf>,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Deserialize, serde::Serialize,
)]
#[clap(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum UpdateSource {
    Release,
    Main,
}

#[derive(Clone, Debug)]
pub struct AuditFilter {
    pub project_id: Option<uuid::Uuid>,
    pub operation: Option<String>,
    pub outcome: Option<String>,
    pub kind: Option<String>,
    pub agent: Option<String>,
    pub session_id: Option<String>,
    pub after: Option<chrono::DateTime<chrono::Utc>>,
    pub before: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AuditProject {
    pub id: uuid::Uuid,
    pub name: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AuditTarget {
    pub kind: String,
    pub id: uuid::Uuid,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AuditEvent {
    pub id: uuid::Uuid,
    pub operation: String,
    pub project: Option<AuditProject>,
    pub target: Option<AuditTarget>,
    pub context: crate::domain::ExecutionContext,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub outcome: String,
    pub exit_code: u8,
    pub revision: Option<i64>,
    pub idempotency_key: Option<String>,
    pub changed_fields: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UserConfig {
    project: Option<String>,
    database: Option<std::path::PathBuf>,
    update_source: Option<UpdateSource>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryConfig {
    project: Option<String>,
    update_source: Option<UpdateSource>,
}

pub struct App {
    database: crate::store::Database,
    idempotency_key: Option<String>,
}

impl App {
    pub const fn new(database: crate::store::Database) -> Self {
        Self {
            database,
            idempotency_key: None,
        }
    }

    pub fn with_idempotency_key(
        mut self,
        key: Option<String>,
    ) -> Result<Self, crate::error::AppError> {
        if let Some(key) = &key {
            crate::domain::validate_idempotency_key(key)?;
        }
        self.idempotency_key = key;
        Ok(self)
    }

    fn idempotency_request(
        &self,
        operation: &str,
        payload: serde_json::Value,
        _context: &crate::domain::ExecutionContext,
    ) -> Result<Option<crate::store::IdempotencyRequest>, crate::error::AppError> {
        let Some(key) = self.idempotency_key.as_deref() else {
            return Ok(None);
        };
        crate::store::IdempotencyRequest::new(key, operation, payload).map(Some)
    }

    fn replay_idempotency<T>(
        &mut self,
        operation: &str,
        request: Option<&crate::store::IdempotencyRequest>,
        project: Option<&str>,
        context: &crate::domain::ExecutionContext,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<T>, crate::error::AppError>
    where
        T: serde::de::DeserializeOwned,
    {
        match self.database.lookup_idempotency(request) {
            Ok(result) => Ok(result),
            Err(error) => Err(self.audited_cli_failure_with_idempotency(
                operation,
                project,
                context,
                error,
                started_at,
                request.map(|request| request.key.as_str()),
            )),
        }
    }

    pub fn audited_cli_failure(
        &mut self,
        operation: &str,
        project: Option<&str>,
        context: &crate::domain::ExecutionContext,
        error: crate::error::AppError,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> crate::error::AppError {
        self.audited_cli_failure_with_idempotency(
            operation, project, context, error, started_at, None,
        )
    }

    fn audited_cli_failure_with_idempotency(
        &mut self,
        operation: &str,
        project: Option<&str>,
        context: &crate::domain::ExecutionContext,
        error: crate::error::AppError,
        started_at: chrono::DateTime<chrono::Utc>,
        idempotency_key: Option<&str>,
    ) -> crate::error::AppError {
        let subject = project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        match self.database.record_failed_operation_with_idempotency(
            operation,
            context,
            &error,
            &subject,
            started_at,
            idempotency_key,
        ) {
            Ok(()) => error,
            Err(audit_error) => Self::failure_audit_error(operation, &error, &audit_error),
        }
    }

    pub fn resolved_context(
        project_argument: Option<String>,
        database_argument: Option<std::path::PathBuf>,
    ) -> Result<ResolvedContext, crate::error::AppError> {
        let user_config_path = Self::user_config_path()?;
        let user_config = Self::read_config::<UserConfig>(&user_config_path)?;
        let directory_config = Self::directory_config()?;

        let project = if let Some(project) = project_argument {
            ResolvedValue {
                value: Some(Self::validate_project_context(project)?),
                source: ContextSource::Argument,
            }
        } else if let Some(project) = Self::environment_string("BETTR_PROJECT")? {
            ResolvedValue {
                value: Some(Self::validate_project_context(project)?),
                source: ContextSource::Environment,
            }
        } else if let Some(project) = directory_config.and_then(|config| config.project) {
            ResolvedValue {
                value: Some(Self::validate_project_context(project)?),
                source: ContextSource::DirectoryConfig,
            }
        } else if let Some(project) = user_config
            .as_ref()
            .and_then(|config| config.project.clone())
        {
            ResolvedValue {
                value: Some(Self::validate_project_context(project)?),
                source: ContextSource::UserConfig,
            }
        } else {
            ResolvedValue {
                value: None,
                source: ContextSource::Default,
            }
        };

        let database = if let Some(database) = database_argument {
            ResolvedValue {
                value: Self::validate_database_path("--database", database)?,
                source: ContextSource::Argument,
            }
        } else if let Some(database) = Self::environment_path("BETTR_DATABASE")? {
            ResolvedValue {
                value: Self::validate_database_path("BETTR_DATABASE", database)?,
                source: ContextSource::Environment,
            }
        } else if let Some(database) = user_config.and_then(|config| config.database) {
            if !database.is_absolute() {
                return Err(crate::error::AppError::InvalidInput(format!(
                    "database in {} must be an absolute path",
                    user_config_path.display()
                )));
            }
            ResolvedValue {
                value: Self::validate_database_path("user config database", database)?,
                source: ContextSource::UserConfig,
            }
        } else {
            let project_directories =
                directories::ProjectDirs::from("", "", "bettr").ok_or_else(|| {
                    crate::error::AppError::InvalidInput(
                        "could not resolve the platform data directory".to_owned(),
                    )
                })?;
            ResolvedValue {
                value: project_directories.data_dir().join("bettr.db"),
                source: ContextSource::Default,
            }
        };

        Ok(ResolvedContext { project, database })
    }

    pub fn resolved_update_source(
        source_argument: Option<UpdateSource>,
    ) -> Result<ResolvedValue<UpdateSource>, crate::error::AppError> {
        let user_config_path = Self::user_config_path()?;
        let user_config = Self::read_config::<UserConfig>(&user_config_path)?;
        let directory_config = Self::directory_config()?;

        if let Some(source) = source_argument {
            return Ok(ResolvedValue {
                value: source,
                source: ContextSource::Argument,
            });
        }
        if let Some(source) = Self::environment_string("BETTR_UPDATE_SOURCE")? {
            return Ok(ResolvedValue {
                value: Self::parse_update_source("BETTR_UPDATE_SOURCE", &source)?,
                source: ContextSource::Environment,
            });
        }
        if let Some(source) = directory_config.and_then(|config| config.update_source) {
            return Ok(ResolvedValue {
                value: source,
                source: ContextSource::DirectoryConfig,
            });
        }
        if let Some(source) = user_config.and_then(|config| config.update_source) {
            return Ok(ResolvedValue {
                value: source,
                source: ContextSource::UserConfig,
            });
        }
        Ok(ResolvedValue {
            value: UpdateSource::Release,
            source: ContextSource::Default,
        })
    }

    pub fn list_audit_events(
        &mut self,
        filter: &AuditFilter,
    ) -> Result<Vec<AuditEvent>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let result = (|| {
            if filter
                .operation
                .as_ref()
                .is_some_and(|operation| operation.trim().is_empty())
            {
                return Err(crate::error::AppError::InvalidInput(
                    "audit operation must not be empty".to_owned(),
                ));
            }
            if filter
                .outcome
                .as_deref()
                .is_some_and(|outcome| !matches!(outcome, "success" | "failure"))
            {
                return Err(crate::error::AppError::InvalidInput(
                    "audit outcome must be success or failure".to_owned(),
                ));
            }
            if filter
                .kind
                .as_deref()
                .is_some_and(|kind| !matches!(kind, "agent" | "human" | "system"))
            {
                return Err(crate::error::AppError::InvalidInput(
                    "audit kind must be agent, human, or system".to_owned(),
                ));
            }
            if filter
                .after
                .zip(filter.before)
                .is_some_and(|(after, before)| after > before)
            {
                return Err(crate::error::AppError::InvalidInput(
                    "audit --after must not be later than --before".to_owned(),
                ));
            }
            self.database.list_audit_events(filter)
        })();
        match result {
            Ok(events) => {
                self.database.record_successful_operation(
                    "audit_list",
                    &context,
                    &crate::store::AuditSubject::default(),
                    &[],
                    started_at,
                )?;
                Ok(events)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "audit_list",
                    &context,
                    &error,
                    &crate::store::AuditSubject::default(),
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "audit_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn verify_audit_jsonl(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::store::jsonl::AuditVerifyResult, crate::error::AppError> {
        self.run_audit_tool(
            "audit_verify",
            |_| crate::store::jsonl::verify_path(path),
            |result| {
                serde_json::json!({
                    "event_count": result.event_count,
                    "first_sequence": result.first_sequence,
                    "last_sequence": result.last_sequence,
                    "valid": result.valid,
                })
                .to_string()
            },
        )
    }

    pub fn archive_audit_jsonl(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::store::jsonl::AuditArchiveResult, crate::error::AppError> {
        self.run_audit_tool(
            "audit_archive",
            |database| database.archive_audit_jsonl(path),
            |result| serde_json::json!({ "archived": result.archived }).to_string(),
        )
    }

    pub fn rebuild_audit_jsonl(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::store::jsonl::AuditRebuildResult, crate::error::AppError> {
        self.run_audit_tool(
            "audit_rebuild",
            |database| database.rebuild_audit_jsonl(path),
            |result| {
                serde_json::json!({
                    "event_count": result.event_count,
                    "first_sequence": result.first_sequence,
                    "last_sequence": result.last_sequence,
                    "rebuilt": result.rebuilt,
                })
                .to_string()
            },
        )
    }

    pub fn redact_issue(
        &mut self,
        project: &str,
        number: i64,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::RedactionResult, crate::error::AppError> {
        self.run_redaction(
            "redact_issue",
            Some(project),
            serde_json::json!({ "project": project, "number": number }),
            context,
            |database| database.redact_issue(project, number, context),
        )
    }

    pub fn redact_comment(
        &mut self,
        id: uuid::Uuid,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::RedactionResult, crate::error::AppError> {
        self.run_redaction(
            "redact_comment",
            None,
            serde_json::json!({ "id": id }),
            context,
            |database| database.redact_comment(id, context),
        )
    }

    pub fn redact_audit(
        &mut self,
        id: uuid::Uuid,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::RedactionResult, crate::error::AppError> {
        self.run_redaction(
            "redact_audit",
            None,
            serde_json::json!({ "id": id }),
            context,
            |database| database.redact_audit(id, context),
        )
    }

    fn run_redaction<F>(
        &mut self,
        operation: &'static str,
        project: Option<&str>,
        payload: serde_json::Value,
        context: &crate::domain::ExecutionContext,
        action: F,
    ) -> Result<crate::domain::RedactionResult, crate::error::AppError>
    where
        F: FnOnce(
            &mut crate::store::Database,
        ) -> Result<crate::domain::RedactionResult, crate::error::AppError>,
    {
        let started_at = chrono::Utc::now();
        let subject = project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let idempotency = self.idempotency_request(operation, payload, context)?;
        let result = if context.kind == crate::domain::InitiatorKind::Human {
            self.database.with_idempotency(idempotency, action)
        } else {
            Err(crate::error::AppError::Conflict(
                "redaction requires a human execution context".to_owned(),
            ))
        };
        match result {
            Ok(result) => Ok(result),
            Err(error) => {
                if let Err(audit_error) = self
                    .database
                    .record_failed_operation(operation, context, &error, &subject, started_at)
                {
                    return Err(Self::failure_audit_error(operation, &error, &audit_error));
                }
                Err(error)
            }
        }
    }

    fn run_audit_tool<T, F, M>(
        &mut self,
        operation: &str,
        action: F,
        metadata: M,
    ) -> Result<T, crate::error::AppError>
    where
        F: FnOnce(&mut crate::store::Database) -> Result<T, crate::error::AppError>,
        M: FnOnce(&T) -> String,
    {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        match action(&mut self.database) {
            Ok(result) => {
                let metadata_json = metadata(&result);
                self.database.record_successful_operation_with_metadata(
                    operation,
                    &context,
                    &crate::store::AuditSubject::default(),
                    &[],
                    started_at,
                    &metadata_json,
                )?;
                Ok(result)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    operation,
                    &context,
                    &error,
                    &crate::store::AuditSubject::default(),
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(operation, &error, &audit_error));
                }
                Err(error)
            }
        }
    }

    pub fn record_context_inspection(&mut self) -> Result<(), crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        self.database.record_successful_operation(
            "context",
            &context,
            &crate::store::AuditSubject::default(),
            &[],
            started_at,
        )
    }

    pub fn create_project(
        &mut self,
        name: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Project, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = self.database.project_audit_subject(name);
        let idempotency = self.idempotency_request(
            "project_create",
            serde_json::json!({ "name": name }),
            context,
        )?;
        match crate::domain::validate_project_name(name).and_then(|()| {
            self.database.with_idempotency(idempotency, |database| {
                database.create_project(name, context)
            })
        }) {
            Ok(project) => Ok(project),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "project_create",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "project_create",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn list_projects(&mut self) -> Result<Vec<crate::domain::Project>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        match self.database.list_projects() {
            Ok(projects) => {
                self.database.record_successful_operation(
                    "project_list",
                    &context,
                    &crate::store::AuditSubject::default(),
                    &[],
                    started_at,
                )?;
                Ok(projects)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "project_list",
                    &context,
                    &error,
                    &crate::store::AuditSubject::default(),
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "project_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn create_issue(
        &mut self,
        project: &str,
        input: crate::domain::NewIssue,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = self.database.project_audit_subject(project);
        let idempotency = self.idempotency_request(
            "issue_create",
            serde_json::json!({ "project": project, "input": input }),
            context,
        )?;
        match input.validate().and_then(|()| {
            self.database.with_idempotency(idempotency, |database| {
                database.create_issue(project, &input, context)
            })
        }) {
            Ok(issue) => Ok(issue),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_create",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_create",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn batch_issues(
        &mut self,
        input_path: &std::path::Path,
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<Vec<crate::domain::BatchResult>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let result = (|| {
            let worktree = crate::worktree::current()?;
            let input = if input_path == std::path::Path::new("-") {
                use std::io::Read as _;
                let mut input = String::new();
                std::io::stdin()
                    .read_to_string(&mut input)
                    .map_err(|error| {
                        crate::error::AppError::InvalidInput(format!(
                            "could not read issue batch input: {error}"
                        ))
                    })?;
                input
            } else {
                std::fs::read_to_string(input_path).map_err(|error| {
                    crate::error::AppError::InvalidInput(format!(
                        "could not read issue batch input: {error}"
                    ))
                })?
            };
            let operations = serde_json::from_str::<Vec<crate::domain::BatchOperation>>(&input)
                .map_err(|error| {
                    crate::error::AppError::InvalidInput(format!(
                        "issue batch input must be a JSON array: {error}"
                    ))
                })?;
            if operations.is_empty() {
                return Err(crate::error::AppError::InvalidInput(
                    "issue batch input must contain at least one operation".to_owned(),
                ));
            }
            let idempotency = self.idempotency_request(
                "issue_batch",
                serde_json::json!({
                    "project": default_project,
                    "operations": operations,
                }),
                context,
            )?;
            if let Some(results) = self.replay_idempotency(
                "issue_batch",
                idempotency.as_ref(),
                default_project,
                context,
                started_at,
            )? {
                return Ok(results);
            }
            self.database.with_idempotency(idempotency, |database| {
                database.batch_issues(&operations, default_project, context, worktree.as_ref())
            })
        })();
        match result {
            Ok(results) => Ok(results),
            Err(error) => {
                let subject = default_project
                    .map(|project| self.database.project_audit_subject(project))
                    .unwrap_or_default();
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_batch",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_batch",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn show_issue(
        &mut self,
        project: &str,
        number: i64,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let crate::store::AuditedResult {
            result: issue_result,
            subject,
        } = self.database.show_issue(project, number);
        let result = if number < 1 {
            Err(crate::error::AppError::InvalidInput(
                "issue number must be positive".to_owned(),
            ))
        } else {
            issue_result
        };
        match result {
            Ok(issue) => {
                let subject = crate::store::AuditSubject::issue(
                    issue.project_id,
                    project,
                    issue.id,
                    issue.revision,
                );
                self.database.record_successful_operation(
                    "issue_show",
                    &context,
                    &subject,
                    &[],
                    started_at,
                )?;
                Ok(issue)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_show",
                    &context,
                    &error,
                    &subject,
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_show",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn show_issue_details(
        &mut self,
        project: &str,
        number: i64,
    ) -> Result<crate::domain::IssueDetails, crate::error::AppError> {
        let issue = self.show_issue(project, number)?;
        let reference = crate::domain::IssueReference {
            project: project.to_owned(),
            number,
        };
        Ok(crate::domain::IssueDetails {
            dependencies: self.database.list_dependencies(&reference)?,
            worktrees: self.database.list_worktrees(&reference)?,
            issue,
        })
    }

    pub fn update_issue(
        &mut self,
        project: &str,
        number: i64,
        expected_revision: i64,
        patch: crate::domain::IssuePatch,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let idempotency = self.idempotency_request(
            "issue_edit",
            serde_json::json!({
                "project": project,
                "number": number,
                "revision": expected_revision,
                "patch": patch,
            }),
            context,
        )?;
        if let Some(issue) = self.replay_idempotency(
            "issue_edit",
            idempotency.as_ref(),
            Some(project),
            context,
            started_at,
        )? {
            return Ok(issue);
        }
        let crate::store::AuditedResult {
            result: issue_result,
            subject,
        } = self.database.show_issue(project, number);
        let result = (|| {
            if number < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue number must be positive".to_owned(),
                ));
            }
            if expected_revision < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue revision must be positive".to_owned(),
                ));
            }
            patch.validate()?;

            let issue = issue_result?;
            if issue.revision != expected_revision {
                return Err(crate::error::AppError::RevisionConflict {
                    current_revision: issue.revision,
                });
            }
            self.database.with_idempotency(idempotency, |database| {
                database.update_issue(&issue, expected_revision, &patch, context)
            })
        })();

        match result {
            Ok(issue) => Ok(issue),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_edit",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_edit",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn add_comment(
        &mut self,
        project: &str,
        number: i64,
        body: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Comment, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let idempotency = self.idempotency_request(
            "issue_comment",
            serde_json::json!({
                "project": project,
                "number": number,
                "body": body,
            }),
            context,
        )?;
        let validation = (|| {
            if number < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue number must be positive".to_owned(),
                ));
            }
            crate::domain::validate_comment_body(body)?;
            Ok(())
        })();
        let crate::store::AuditedResult { result, subject } = match validation {
            Ok(()) => self.database.with_idempotency(idempotency, |database| {
                database.add_comment(project, number, body, context)
            }),
            Err(error) => crate::store::AuditedResult {
                result: Err(error),
                subject: self.database.project_audit_subject(project),
            },
        };

        match result {
            Ok(comment) => Ok(comment),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_comment",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_comment",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn issue_history(
        &mut self,
        project: &str,
        number: i64,
    ) -> Result<Vec<crate::domain::DomainEvent>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let crate::store::AuditedResult { result, subject } = if number < 1 {
            crate::store::AuditedResult {
                result: Err(crate::error::AppError::InvalidInput(
                    "issue number must be positive".to_owned(),
                )),
                subject: self.database.project_audit_subject(project),
            }
        } else {
            self.database.issue_history(project, number)
        };
        match result {
            Ok((issue, history)) => {
                let subject = crate::store::AuditSubject::issue(
                    issue.project_id,
                    project,
                    issue.id,
                    issue.revision,
                );
                self.database.record_successful_operation(
                    "issue_history",
                    &context,
                    &subject,
                    &[],
                    started_at,
                )?;
                Ok(history)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_history",
                    &context,
                    &error,
                    &subject,
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_history",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn add_dependency(
        &mut self,
        blocker: &str,
        blocked: &str,
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueDependency, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let blocker = crate::domain::IssueReference::parse(blocker, default_project)?;
            let blocked = crate::domain::IssueReference::parse(blocked, default_project)?;
            let idempotency = self.idempotency_request(
                "issue_dependency_add",
                serde_json::json!({
                    "blocker": blocker.label(),
                    "blocked": blocked.label(),
                }),
                context,
            )?;
            self.database.with_idempotency(idempotency, |database| {
                database.add_dependency(&blocker, &blocked, context)
            })
        })();
        match result {
            Ok(relation) => Ok(relation),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_dependency_add",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_dependency_add",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn remove_dependency(
        &mut self,
        blocker: &str,
        blocked: &str,
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueDependency, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let blocker = crate::domain::IssueReference::parse(blocker, default_project)?;
            let blocked = crate::domain::IssueReference::parse(blocked, default_project)?;
            let idempotency = self.idempotency_request(
                "issue_dependency_remove",
                serde_json::json!({
                    "blocker": blocker.label(),
                    "blocked": blocked.label(),
                }),
                context,
            )?;
            self.database.with_idempotency(idempotency, |database| {
                database.remove_dependency(&blocker, &blocked, context)
            })
        })();
        match result {
            Ok(relation) => Ok(relation),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_dependency_remove",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_dependency_remove",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn list_dependencies(
        &mut self,
        reference: &str,
        default_project: Option<&str>,
    ) -> Result<Vec<crate::domain::IssueDependency>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let reference = crate::domain::IssueReference::parse(reference, default_project)?;
            self.database.list_dependencies(&reference)
        })();
        match result {
            Ok(relations) => {
                self.database.record_successful_operation(
                    "issue_dependency_list",
                    &context,
                    &subject,
                    &[],
                    started_at,
                )?;
                Ok(relations)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_dependency_list",
                    &context,
                    &error,
                    &subject,
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_dependency_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn add_worktree(
        &mut self,
        reference: &str,
        path: Option<&std::path::Path>,
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueWorktree, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let reference = crate::domain::IssueReference::parse(reference, default_project)?;
            let worktree = match path {
                Some(path) => crate::worktree::from_path(path)?,
                None => crate::worktree::current()?.ok_or_else(|| {
                    crate::error::AppError::InvalidInput(
                        "current directory is not inside a Git worktree".to_owned(),
                    )
                })?,
            };
            let idempotency = self.idempotency_request(
                "issue_worktree_add",
                serde_json::json!({
                    "reference": reference.label(),
                    "path": worktree.path,
                }),
                context,
            )?;
            self.database.with_idempotency(idempotency, |database| {
                database.add_worktree(&reference, &worktree, context)
            })
        })();
        match result {
            Ok(worktree) => Ok(worktree),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_worktree_add",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_worktree_add",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn remove_worktree(
        &mut self,
        reference: &str,
        path: Option<&std::path::Path>,
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueWorktree, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let reference = crate::domain::IssueReference::parse(reference, default_project)?;
            let path = match path {
                Some(path) => Self::normalize_worktree_path(path)?,
                None => {
                    crate::worktree::current()?
                        .ok_or_else(|| {
                            crate::error::AppError::InvalidInput(
                                "current directory is not inside a Git worktree".to_owned(),
                            )
                        })?
                        .path
                }
            };
            let idempotency = self.idempotency_request(
                "issue_worktree_remove",
                serde_json::json!({
                    "reference": reference.label(),
                    "path": path,
                }),
                context,
            )?;
            self.database.with_idempotency(idempotency, |database| {
                database.remove_worktree(&reference, &path, context)
            })
        })();
        match result {
            Ok(worktree) => Ok(worktree),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_worktree_remove",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_worktree_remove",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn list_worktrees(
        &mut self,
        reference: &str,
        default_project: Option<&str>,
    ) -> Result<Vec<crate::domain::IssueWorktree>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let reference = crate::domain::IssueReference::parse(reference, default_project)?;
            self.database.list_worktrees(&reference)
        })();
        match result {
            Ok(worktrees) => {
                self.database.record_successful_operation(
                    "issue_worktree_list",
                    &context,
                    &subject,
                    &[],
                    started_at,
                )?;
                Ok(worktrees)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_worktree_list",
                    &context,
                    &error,
                    &subject,
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_worktree_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    fn normalize_worktree_path(path: &std::path::Path) -> Result<String, crate::error::AppError> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            let current = std::env::current_dir().map_err(|error| {
                crate::error::AppError::Internal(format!(
                    "could not resolve current directory: {error}"
                ))
            })?;
            current.join(path)
        };
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        path.to_str().map(str::to_owned).ok_or_else(|| {
            crate::error::AppError::InvalidInput("worktree path must be valid Unicode".to_owned())
        })
    }

    pub fn set_parent(
        &mut self,
        child: &str,
        parent: &str,
        default_project: Option<&str>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::IssueParent, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let child = crate::domain::IssueReference::parse(child, default_project)?;
            let parent = crate::domain::IssueReference::parse(parent, default_project)?;
            let idempotency = self.idempotency_request(
                "issue_parent_set",
                serde_json::json!({
                    "child": child.label(),
                    "parent": parent.label(),
                }),
                context,
            )?;
            self.database.with_idempotency(idempotency, |database| {
                database.set_parent(&child, &parent, context)
            })
        })();
        match result {
            Ok(relation) => Ok(relation),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_parent_set",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_parent_set",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn list_parent(
        &mut self,
        reference: &str,
        default_project: Option<&str>,
    ) -> Result<Vec<crate::domain::IssueParent>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let subject = default_project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let result = (|| {
            let reference = crate::domain::IssueReference::parse(reference, default_project)?;
            self.database.list_parent(&reference)
        })();
        match result {
            Ok(relations) => {
                self.database.record_successful_operation(
                    "issue_parent_list",
                    &context,
                    &subject,
                    &[],
                    started_at,
                )?;
                Ok(relations)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_parent_list",
                    &context,
                    &error,
                    &subject,
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_parent_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn claim_issue(
        &mut self,
        project: Option<&str>,
        number: Option<i64>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::ClaimedIssue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let worktree = crate::worktree::current()?;
        let subject = project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        let idempotency = self.idempotency_request(
            "issue_claim",
            serde_json::json!({ "project": project, "number": number }),
            context,
        )?;
        match self.database.with_idempotency(idempotency, |database| {
            database.claim_issue(project, number, context, worktree.as_ref())
        }) {
            Ok(claimed) => Ok(claimed),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_claim",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_claim",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn request_decision(
        &mut self,
        project: &str,
        number: i64,
        input: &crate::domain::DecisionRequestInput,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::DecisionRequest, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let subject = self.database.project_audit_subject(project);
        let idempotency = self.idempotency_request(
            "decision_request",
            serde_json::json!({
                "project": project,
                "number": number,
                "blocker": input.blocker,
                "question": input.question,
                "options": input.options,
                "recommendation": input.recommendation,
                "resume_condition": input.resume_condition,
                "background": input.background,
            }),
            context,
        )?;
        let result = (|| {
            if number < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue number must be positive".to_owned(),
                ));
            }
            crate::domain::validate_decision_text("decision question", &input.question)?;
            crate::domain::validate_decision_text("decision blocker", &input.blocker)?;
            crate::domain::validate_decision_options(&input.options)?;
            crate::domain::validate_decision_text(
                "decision recommendation",
                &input.recommendation,
            )?;
            crate::domain::validate_decision_text(
                "decision resume condition",
                &input.resume_condition,
            )?;
            crate::domain::validate_decision_text("decision background", &input.background)?;
            self.database.with_idempotency(idempotency, |database| {
                database.request_decision(project, number, input, context)
            })
        })();
        match result {
            Ok(request) => Ok(request),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "decision_request",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "decision_request",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn resolve_decision(
        &mut self,
        request_id: &str,
        answer: &str,
        expected_revision: Option<i64>,
        resolution: crate::domain::DecisionResolutionInput,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::DecisionRequest, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let idempotency = self.idempotency_request(
            "decision_resolve",
            serde_json::json!({
                "request_id": request_id,
                "answer": answer,
                "expected_revision": expected_revision,
                "resolution": resolution,
            }),
            context,
        )?;
        let result = (|| {
            let request_id = uuid::Uuid::parse_str(request_id).map_err(|_| {
                crate::error::AppError::InvalidInput(
                    "decision request id must be a UUID".to_owned(),
                )
            })?;
            if expected_revision.is_some_and(|revision| revision < 1) {
                return Err(crate::error::AppError::InvalidInput(
                    "issue revision must be positive".to_owned(),
                ));
            }
            crate::domain::validate_decision_text("decision answer", answer)?;
            self.database.with_idempotency(idempotency, |database| {
                database.resolve_decision(
                    request_id,
                    answer,
                    expected_revision,
                    resolution,
                    context,
                )
            })
        })();
        match result {
            Ok(request) => Ok(request),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "decision_resolve",
                    context,
                    &error,
                    &crate::store::AuditSubject::default(),
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "decision_resolve",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn list_decisions(
        &mut self,
        project: &str,
        number: i64,
    ) -> Result<Vec<crate::domain::DecisionRequest>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let subject = self.database.project_audit_subject(project);
        let result = if number < 1 {
            Err(crate::error::AppError::InvalidInput(
                "issue number must be positive".to_owned(),
            ))
        } else {
            self.database.list_decisions(project, number)
        };
        match result {
            Ok(decisions) => {
                self.database.record_successful_operation(
                    "decision_list",
                    &context,
                    &subject,
                    &[],
                    started_at,
                )?;
                Ok(decisions)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "decision_list",
                    &context,
                    &error,
                    &subject,
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "decision_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn heartbeat_issue(
        &mut self,
        project: &str,
        number: i64,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::ClaimedIssue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let idempotency = self.idempotency_request(
            "issue_heartbeat",
            serde_json::json!({ "project": project, "number": number }),
            context,
        )?;
        if let Some(claimed) = self.replay_idempotency(
            "issue_heartbeat",
            idempotency.as_ref(),
            Some(project),
            context,
            started_at,
        )? {
            return Ok(claimed);
        }
        let crate::store::AuditedResult {
            result: issue_result,
            subject,
        } = self.database.show_issue(project, number);
        let result = (|| {
            if number < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue number must be positive".to_owned(),
                ));
            }
            let issue = issue_result?;
            self.database.with_idempotency(idempotency, |database| {
                database.heartbeat_issue(&issue, context)
            })
        })();
        match result {
            Ok(claimed) => Ok(claimed),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_heartbeat",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_heartbeat",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn takeover_issue(
        &mut self,
        project: &str,
        number: i64,
        reason: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::ClaimedIssue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let idempotency = self.idempotency_request(
            "issue_takeover",
            serde_json::json!({
                "project": project,
                "number": number,
                "reason": reason,
            }),
            context,
        )?;
        if let Some(claimed) = self.replay_idempotency(
            "issue_takeover",
            idempotency.as_ref(),
            Some(project),
            context,
            started_at,
        )? {
            return Ok(claimed);
        }
        let crate::store::AuditedResult {
            result: issue_result,
            subject,
        } = self.database.show_issue(project, number);
        let result = (|| {
            if number < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue number must be positive".to_owned(),
                ));
            }
            let issue = issue_result?;
            self.database.with_idempotency(idempotency, |database| {
                database.takeover_issue(&issue, reason, context)
            })
        })();
        match result {
            Ok(claimed) => Ok(claimed),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    "issue_takeover",
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_takeover",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn transition_issue(
        &mut self,
        project: &str,
        number: i64,
        expected_revision: i64,
        operation: &'static str,
        transition: Result<crate::domain::Transition, crate::domain::DomainError>,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let worktree = if operation == "issue_start" {
            crate::worktree::current()?
        } else {
            None
        };
        let transition_payload = transition.as_ref().map_or_else(
            |_| serde_json::Value::Null,
            crate::domain::Transition::idempotency_payload,
        );
        let idempotency = self.idempotency_request(
            operation,
            serde_json::json!({
                "project": project,
                "number": number,
                "revision": expected_revision,
                "transition": transition_payload,
            }),
            context,
        )?;
        if let Some(issue) = self.replay_idempotency(
            operation,
            idempotency.as_ref(),
            Some(project),
            context,
            started_at,
        )? {
            return Ok(issue);
        }
        let crate::store::AuditedResult {
            result: issue_result,
            subject,
        } = self.database.show_issue(project, number);
        let result = (|| {
            let transition = transition?;
            if number < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue number must be positive".to_owned(),
                ));
            }
            if expected_revision < 1 {
                return Err(crate::error::AppError::InvalidInput(
                    "issue revision must be positive".to_owned(),
                ));
            }

            let issue = issue_result?;
            if issue.revision != expected_revision {
                return Err(crate::error::AppError::RevisionConflict {
                    current_revision: issue.revision,
                });
            }
            let target_state = issue.state.apply(&transition)?;
            self.database.with_idempotency(idempotency, |database| {
                database.transition_issue(
                    &issue,
                    expected_revision,
                    &transition,
                    target_state,
                    context,
                    worktree.as_ref(),
                )
            })
        })();

        match result {
            Ok(issue) => Ok(issue),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation_with_idempotency(
                    operation,
                    context,
                    &error,
                    &subject,
                    started_at,
                    self.idempotency_key.as_deref(),
                ) {
                    return Err(Self::failure_audit_error(operation, &error, &audit_error));
                }
                Err(error)
            }
        }
    }

    pub fn list_issues(
        &mut self,
        filter: &crate::domain::IssueFilter,
    ) -> Result<Vec<crate::domain::IssueListItem>, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let subject = filter
            .projects
            .first()
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        match self.database.list_issues(filter) {
            Ok(issues) => {
                self.database.record_successful_operation(
                    "issue_list",
                    &context,
                    &subject,
                    &[],
                    started_at,
                )?;
                Ok(issues)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_list",
                    &context,
                    &error,
                    &subject,
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "issue_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn status(&mut self) -> Result<crate::domain::Status, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let filter = crate::domain::IssueFilter {
            projects: Vec::new(),
            states: Vec::new(),
            priorities: Vec::new(),
            assignee: None,
            updated_after: None,
            query: None,
            include_done: true,
        };
        let result = (|| {
            let issues = self.database.list_issues(&filter)?;
            let stale = self.database.list_stale_issues(chrono::Utc::now())?;
            let attention = self.database.list_attention_issues()?;
            Ok((issues, stale, attention))
        })();
        match result {
            Ok((issues, stale, attention)) => {
                let mut status = crate::domain::Status {
                    attention,
                    stale,
                    blocked: Vec::new(),
                    recently_completed: Vec::new(),
                    active: Vec::new(),
                };
                for issue in issues {
                    if status
                        .attention
                        .iter()
                        .any(|attention| attention.issue.id == issue.issue.id)
                    {
                        continue;
                    }
                    match issue.issue.state {
                        crate::domain::IssueState::Blocked => status.blocked.push(issue),
                        crate::domain::IssueState::Done | crate::domain::IssueState::Cancelled => {
                            status.recently_completed.push(issue);
                        }
                        crate::domain::IssueState::Todo | crate::domain::IssueState::InProgress => {
                            if status
                                .stale
                                .iter()
                                .any(|stale| stale.issue.id == issue.issue.id)
                            {
                                continue;
                            }
                            status.active.push(issue)
                        }
                    }
                }
                self.database.record_successful_operation(
                    "status",
                    &context,
                    &crate::store::AuditSubject::default(),
                    &[],
                    started_at,
                )?;
                Ok(status)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "status",
                    &context,
                    &error,
                    &crate::store::AuditSubject::default(),
                    started_at,
                ) {
                    return Err(Self::failure_audit_error("status", &error, &audit_error));
                }
                Err(error)
            }
        }
    }

    pub fn list_events(
        &mut self,
        after: i64,
        limit: Option<i64>,
        include_issue: bool,
    ) -> Result<crate::domain::EventPage, crate::error::AppError> {
        let started_at = chrono::Utc::now();
        let context = crate::domain::ExecutionContext::resolve()?;
        let result = (|| {
            if after < 0 {
                return Err(crate::error::AppError::InvalidInput(
                    "event cursor must not be negative".to_owned(),
                ));
            }
            let limit = limit.unwrap_or(100);
            if !(1..=500).contains(&limit) {
                return Err(crate::error::AppError::InvalidInput(
                    "event limit must be between 1 and 500".to_owned(),
                ));
            }
            self.database
                .list_events(after, limit as usize, include_issue)
        })();
        match result {
            Ok(page) => {
                self.database.record_successful_operation(
                    "event_list",
                    &context,
                    &crate::store::AuditSubject::default(),
                    &[],
                    started_at,
                )?;
                Ok(page)
            }
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "event_list",
                    &context,
                    &error,
                    &crate::store::AuditSubject::default(),
                    started_at,
                ) {
                    return Err(Self::failure_audit_error(
                        "event_list",
                        &error,
                        &audit_error,
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn capabilities() -> crate::domain::CapabilitySet {
        crate::domain::CapabilitySet {
            json_contract_version: 1,
            cli_version: env!("CARGO_PKG_VERSION"),
            capabilities: [
                ("issue_dependencies", true),
                ("issue_worktrees", true),
                ("issue_parent", true),
                ("issue_claim", true),
                ("issue_lease", true),
                ("human_decisions", true),
                ("event_cursor", true),
                ("capabilities", true),
                ("idempotency", true),
                ("audit_jsonl", true),
                ("redaction", true),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn user_config_path() -> Result<std::path::PathBuf, crate::error::AppError> {
        directories::ProjectDirs::from("", "", "bettr")
            .map(|directories| directories.config_dir().join("config.toml"))
            .ok_or_else(|| {
                crate::error::AppError::InvalidInput(
                    "could not resolve the platform config directory".to_owned(),
                )
            })
    }

    fn directory_config() -> Result<Option<DirectoryConfig>, crate::error::AppError> {
        let current_directory = std::env::current_dir().map_err(|error| {
            crate::error::AppError::InvalidInput(format!(
                "could not resolve the current directory: {error}"
            ))
        })?;
        for directory in current_directory.ancestors() {
            let path = directory.join(".bettr.toml");
            if path.exists() {
                return Self::read_config(&path);
            }
        }
        Ok(None)
    }

    fn read_config<T>(path: &std::path::Path) -> Result<Option<T>, crate::error::AppError>
    where
        T: serde::de::DeserializeOwned,
    {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(crate::error::AppError::InvalidInput(format!(
                    "could not read config {}: {error}",
                    path.display()
                )));
            }
        };
        toml::from_str(&contents).map(Some).map_err(|error| {
            crate::error::AppError::InvalidInput(format!(
                "invalid config {}: {error}",
                path.display()
            ))
        })
    }

    fn environment_string(name: &str) -> Result<Option<String>, crate::error::AppError> {
        match std::env::var_os(name) {
            Some(value) => value.into_string().map(Some).map_err(|_| {
                crate::error::AppError::InvalidInput(format!("{name} must be valid Unicode"))
            }),
            None => Ok(None),
        }
    }

    fn environment_path(name: &str) -> Result<Option<std::path::PathBuf>, crate::error::AppError> {
        Self::environment_string(name).map(|value| value.map(std::path::PathBuf::from))
    }

    fn parse_update_source(
        name: &str,
        value: &str,
    ) -> Result<UpdateSource, crate::error::AppError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "release" => Ok(UpdateSource::Release),
            "main" => Ok(UpdateSource::Main),
            _ => Err(crate::error::AppError::InvalidInput(format!(
                "{name} must be release or main"
            ))),
        }
    }

    fn validate_project_context(project: String) -> Result<String, crate::error::AppError> {
        crate::domain::validate_project_name(&project)?;
        Ok(project)
    }

    fn validate_database_path(
        name: &str,
        database: std::path::PathBuf,
    ) -> Result<std::path::PathBuf, crate::error::AppError> {
        if database.as_os_str().is_empty() {
            return Err(crate::error::AppError::InvalidInput(format!(
                "{name} must not be empty"
            )));
        }
        Ok(database)
    }

    fn failure_audit_error(
        operation: &str,
        original_error: &crate::error::AppError,
        audit_error: &crate::error::AppError,
    ) -> crate::error::AppError {
        crate::error::AppError::Internal(format!(
            "failed to persist failure audit for {operation} after {} ({})",
            original_error.code(),
            audit_error.code()
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn update_source_configuration_accepts_release_and_main() {
        let config = toml::from_str::<super::UserConfig>("update_source = \"main\"\n").unwrap();
        assert_eq!(config.update_source, Some(super::UpdateSource::Main));
        assert_eq!(
            super::App::parse_update_source("BETTR_UPDATE_SOURCE", " RELEASE ").unwrap(),
            super::UpdateSource::Release
        );
    }
}
