pub struct Database {
    connection: rusqlite::Connection,
}

impl Database {
    pub fn initialize(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        Self::create_parent_directory(path)?;

        let file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(crate::error::AppError::DatabaseAlreadyInitialized);
            }
            Err(error) => return Err(crate::error::AppError::Internal(error.to_string())),
        };
        drop(file);

        let database = Self::open_existing(path);
        let mut database = match database {
            Ok(database) => database,
            Err(error) => {
                return Err(Self::cleanup_after_initialization_failure(path, error));
            }
        };

        if let Err(error) = database.initialize_schema() {
            drop(database);
            return Err(Self::cleanup_after_initialization_failure(path, error));
        }

        Ok(database)
    }

    pub fn open(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        if !path.is_file() {
            return Err(crate::error::AppError::DatabaseNotInitialized);
        }

        Self::open_existing(path)
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
            .map_err(Self::database_error)?;

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
                if code.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(crate::error::AppError::ProjectNameConflict);
            }
            Err(error) => return Err(Self::database_error(error)),
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
            .map_err(Self::database_error)?;
        let project_id = project.id.to_string();
        Self::insert_audit_event(
            &transaction,
            "project_create",
            true,
            context,
            Some(("project", &project_id)),
            "{}",
        )?;
        transaction.commit().map_err(Self::database_error)?;

        Ok(project)
    }

    pub fn list_projects(&self) -> Result<Vec<crate::domain::Project>, crate::error::AppError> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, archived, created_at FROM projects ORDER BY name ASC")
            .map_err(Self::database_error)?;
        let mut rows = statement.query([]).map_err(Self::database_error)?;
        let mut projects = Vec::new();
        while let Some(row) = rows.next().map_err(Self::database_error)? {
            let id: String = row.get(0).map_err(Self::database_error)?;
            let created_at: String = row.get(3).map_err(Self::database_error)?;
            projects.push(crate::domain::Project {
                id: uuid::Uuid::parse_str(&id)
                    .map_err(|error| crate::error::AppError::Internal(error.to_string()))?,
                name: row.get(1).map_err(Self::database_error)?,
                archived: row.get::<_, i64>(2).map_err(Self::database_error)? != 0,
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
        for attempt in 0..3 {
            match self.create_issue_once(project_name, input, context) {
                Err(crate::error::AppError::DatabaseBusy(_)) if attempt < 2 => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                result => return result,
            }
        }
        unreachable!("the retry loop always returns")
    }

    pub fn show_issue(
        &self,
        project_name: &str,
        number: i64,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let project_id = self.project_id(project_name)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, project_id, number, title, body, state, priority, assignee_kind,
                        assignee_name, revision, created_at, updated_at
                 FROM issues WHERE project_id = ?1 AND number = ?2",
            )
            .map_err(Self::database_error)?;
        statement
            .query_row(
                rusqlite::params![project_id.to_string(), number],
                Self::issue_from_row,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    crate::error::AppError::NotFound("issue not found".to_owned())
                }
                error => Self::database_error(error),
            })
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
            .map_err(Self::database_error)?;
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
            .map_err(Self::database_error)?;
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
                    error => Self::database_error(error),
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
            .map_err(Self::database_error)?;
        let issue_id = updated_issue.id.to_string();
        let audit_metadata = serde_json::json!({ "revision": updated_issue.revision }).to_string();
        Self::insert_audit_event(
            &transaction,
            transition.operation(),
            true,
            context,
            Some(("issue", &issue_id)),
            &audit_metadata,
        )?;
        transaction.commit().map_err(Self::database_error)?;

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
            .map_err(Self::database_error)?;
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
            .map_err(Self::database_error)?;
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
                    error => Self::database_error(error),
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
            .map_err(Self::database_error)?;
        let issue_id = updated_issue.id.to_string();
        let audit_metadata = serde_json::json!({ "revision": updated_issue.revision }).to_string();
        Self::insert_audit_event(
            &transaction,
            "issue_edit",
            true,
            context,
            Some(("issue", &issue_id)),
            &audit_metadata,
        )?;
        transaction.commit().map_err(Self::database_error)?;

        Ok(updated_issue)
    }

    pub fn add_comment(
        &mut self,
        issue: &crate::domain::Issue,
        body: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Comment, crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(Self::database_error)?;
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
            .map_err(Self::database_error)?;
        transaction
            .execute(
                "UPDATE issues SET updated_at = ?1 WHERE id = ?2",
                rusqlite::params![
                    comment.created_at.to_rfc3339(),
                    comment.issue_id.to_string()
                ],
            )
            .map_err(Self::database_error)?;
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
            .map_err(Self::database_error)?;
        let issue_id = issue.id.to_string();
        let audit_metadata = serde_json::json!({
            "comment_id": comment.id,
            "revision": issue.revision,
        })
        .to_string();
        Self::insert_audit_event(
            &transaction,
            "issue_comment",
            true,
            context,
            Some(("issue", &issue_id)),
            &audit_metadata,
        )?;
        transaction.commit().map_err(Self::database_error)?;

        Ok(comment)
    }

    pub fn issue_history(
        &self,
        issue_id: uuid::Uuid,
    ) -> Result<Vec<crate::domain::DomainEvent>, crate::error::AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT sequence, event_type, metadata_json, created_at
                 FROM domain_events WHERE issue_id = ?1 ORDER BY sequence ASC",
            )
            .map_err(Self::database_error)?;
        let mut rows = statement
            .query([issue_id.to_string()])
            .map_err(Self::database_error)?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(Self::database_error)? {
            let metadata_json: String = row.get(2).map_err(Self::database_error)?;
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
            let created_at: String = row.get(3).map_err(Self::database_error)?;
            events.push(crate::domain::DomainEvent {
                sequence: row.get(0).map_err(Self::database_error)?,
                event_type: row.get(1).map_err(Self::database_error)?,
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
                    WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2
                    WHEN 'low' THEN 3 ELSE 4
                END ASC,
                i.created_at ASC,
                p.name ASC,
                i.number ASC",
        );

        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(Self::database_error)?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(parameters.iter()))
            .map_err(Self::database_error)?;
        let mut issues = Vec::new();
        while let Some(row) = rows.next().map_err(Self::database_error)? {
            issues.push(crate::domain::IssueListItem {
                project: row.get(0).map_err(Self::database_error)?,
                issue: Self::issue_from_row_at(row, 1).map_err(Self::database_error)?,
            });
        }
        Ok(issues)
    }

    pub fn record_successful_operation(
        &mut self,
        operation: &str,
        context: &crate::domain::ExecutionContext,
        target: Option<(&str, &str)>,
    ) -> Result<(), crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(Self::database_error)?;
        Self::insert_audit_event(&transaction, operation, true, context, target, "{}")?;
        transaction.commit().map_err(Self::database_error)
    }

    pub fn record_failed_operation(
        &mut self,
        operation: &str,
        context: &crate::domain::ExecutionContext,
        error: &crate::error::AppError,
    ) -> Result<(), crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(Self::database_error)?;
        let metadata_json = serde_json::json!({ "error_code": error.code() }).to_string();
        Self::insert_audit_event(
            &transaction,
            operation,
            false,
            context,
            None,
            &metadata_json,
        )?;
        transaction.commit().map_err(Self::database_error)
    }

    fn insert_audit_event(
        transaction: &rusqlite::Transaction<'_>,
        operation: &str,
        success: bool,
        context: &crate::domain::ExecutionContext,
        target: Option<(&str, &str)>,
        metadata_json: &str,
    ) -> Result<(), crate::error::AppError> {
        let (target_type, target_id) = target.unzip();
        transaction
            .execute(
                "INSERT INTO audit_events (
                    id, occurred_at, operation, success, initiator_kind, initiator_name,
                    session_id, target_type, target_id, metadata_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    chrono::Utc::now().to_rfc3339(),
                    operation,
                    i64::from(success),
                    context.kind.as_str(),
                    context.initiator_name(),
                    context.session_id,
                    target_type,
                    target_id,
                    metadata_json,
                ],
            )
            .map_err(Self::database_error)?;
        Ok(())
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
            .map_err(Self::database_error)?;
        let project_id = Self::project_id_in_transaction(&transaction, project_name)?;
        let number = transaction
            .query_row(
                "SELECT COALESCE(MAX(number), 0) + 1 FROM issues WHERE project_id = ?1",
                [project_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(Self::database_error)?;
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
            .map_err(Self::database_error)?;
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
            .map_err(Self::database_error)?;
        let issue_id = issue.id.to_string();
        Self::insert_audit_event(
            &transaction,
            "issue_create",
            true,
            context,
            Some(("issue", &issue_id)),
            "{}",
        )?;
        transaction.commit().map_err(Self::database_error)?;
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
                error => Self::database_error(error),
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
                error => Self::database_error(error),
            })?;
        uuid::Uuid::parse_str(&id)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))
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

    fn open_existing(path: &std::path::Path) -> Result<Self, crate::error::AppError> {
        let connection = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(Self::database_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(Self::database_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(Self::database_error)?;

        Ok(Self { connection })
    }

    fn initialize_schema(&mut self) -> Result<(), crate::error::AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(Self::database_error)?;
        transaction
            .execute_batch(include_str!("schema.sql"))
            .map_err(Self::database_error)?;
        transaction
            .execute(
                "INSERT INTO audit_events (id, occurred_at, operation, success, metadata_json)
                 VALUES (
                    lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' ||
                    lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' ||
                    lower(hex(randomblob(6))),
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    'init',
                    1,
                    '{}'
                 )",
                [],
            )
            .map_err(Self::database_error)?;
        transaction.commit().map_err(Self::database_error)
    }

    fn database_error(error: rusqlite::Error) -> crate::error::AppError {
        if let rusqlite::Error::SqliteFailure(code, _) = &error
            && matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
        {
            return crate::error::AppError::DatabaseBusy(error.to_string());
        }

        crate::error::AppError::Internal(error.to_string())
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
}
