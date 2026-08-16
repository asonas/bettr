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
    pub changed_fields: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UserConfig {
    project: Option<String>,
    database: Option<std::path::PathBuf>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryConfig {
    project: Option<String>,
}

pub struct App {
    database: crate::store::Database,
}

impl App {
    pub const fn new(database: crate::store::Database) -> Self {
        Self { database }
    }

    pub fn audited_cli_failure(
        &mut self,
        operation: &str,
        project: Option<&str>,
        context: &crate::domain::ExecutionContext,
        error: crate::error::AppError,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> crate::error::AppError {
        let subject = project
            .map(|project| self.database.project_audit_subject(project))
            .unwrap_or_default();
        match self
            .database
            .record_failed_operation(operation, context, &error, &subject, started_at)
        {
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
        match crate::domain::validate_project_name(name)
            .and_then(|()| self.database.create_project(name, context))
        {
            Ok(project) => Ok(project),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "project_create",
                    context,
                    &error,
                    &subject,
                    started_at,
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
        match input
            .validate()
            .and_then(|()| self.database.create_issue(project, &input, context))
        {
            Ok(issue) => Ok(issue),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_create",
                    context,
                    &error,
                    &subject,
                    started_at,
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

    pub fn update_issue(
        &mut self,
        project: &str,
        number: i64,
        expected_revision: i64,
        patch: crate::domain::IssuePatch,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let started_at = chrono::Utc::now();
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
            self.database
                .update_issue(&issue, expected_revision, &patch, context)
        })();

        match result {
            Ok(issue) => Ok(issue),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_edit",
                    context,
                    &error,
                    &subject,
                    started_at,
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
            Ok(()) => self.database.add_comment(project, number, body, context),
            Err(error) => crate::store::AuditedResult {
                result: Err(error),
                subject: self.database.project_audit_subject(project),
            },
        };

        match result {
            Ok(comment) => Ok(comment),
            Err(error) => {
                if let Err(audit_error) = self.database.record_failed_operation(
                    "issue_comment",
                    context,
                    &error,
                    &subject,
                    started_at,
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
            self.database.transition_issue(
                &issue,
                expected_revision,
                &transition,
                target_state,
                context,
            )
        })();

        match result {
            Ok(issue) => Ok(issue),
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
        match self.database.list_issues(&filter) {
            Ok(issues) => {
                let mut status = crate::domain::Status {
                    attention: Vec::new(),
                    stale: Vec::new(),
                    blocked: Vec::new(),
                    recently_completed: Vec::new(),
                    active: Vec::new(),
                };
                for issue in issues {
                    match issue.issue.state {
                        crate::domain::IssueState::Blocked => status.blocked.push(issue),
                        crate::domain::IssueState::Done | crate::domain::IssueState::Cancelled => {
                            status.recently_completed.push(issue);
                        }
                        crate::domain::IssueState::Todo | crate::domain::IssueState::InProgress => {
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
