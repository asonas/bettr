pub struct App {
    database: crate::store::Database,
}

impl App {
    pub const fn new(database: crate::store::Database) -> Self {
        Self { database }
    }

    pub fn create_project(
        &mut self,
        name: &str,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Project, crate::error::AppError> {
        match crate::domain::validate_project_name(name)
            .and_then(|()| self.database.create_project(name, context))
        {
            Ok(project) => Ok(project),
            Err(error) => {
                if let Err(audit_error) =
                    self.database
                        .record_failed_operation("project_create", context, &error)
                {
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
        let context = crate::domain::ExecutionContext::resolve()?;
        match self.database.list_projects() {
            Ok(projects) => {
                self.database
                    .record_successful_operation("project_list", &context, None)?;
                Ok(projects)
            }
            Err(error) => {
                if let Err(audit_error) =
                    self.database
                        .record_failed_operation("project_list", &context, &error)
                {
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
        match input
            .validate()
            .and_then(|()| self.database.create_issue(project, &input, context))
        {
            Ok(issue) => Ok(issue),
            Err(error) => {
                if let Err(audit_error) =
                    self.database
                        .record_failed_operation("issue_create", context, &error)
                {
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
        if number < 1 {
            return Err(crate::error::AppError::InvalidInput(
                "issue number must be positive".to_owned(),
            ));
        }

        let context = crate::domain::ExecutionContext::resolve()?;
        match self.database.show_issue(project, number) {
            Ok(issue) => {
                let issue_id = issue.id.to_string();
                self.database.record_successful_operation(
                    "issue_show",
                    &context,
                    Some(("issue", &issue_id)),
                )?;
                Ok(issue)
            }
            Err(error) => {
                if let Err(audit_error) =
                    self.database
                        .record_failed_operation("issue_show", &context, &error)
                {
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

    pub fn transition_issue(
        &mut self,
        project: &str,
        number: i64,
        expected_revision: i64,
        transition: crate::domain::Transition,
        context: &crate::domain::ExecutionContext,
    ) -> Result<crate::domain::Issue, crate::error::AppError> {
        let operation = transition.operation();
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

            let issue = self.database.show_issue(project, number)?;
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
                    .record_failed_operation(operation, context, &error)
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
        let context = crate::domain::ExecutionContext::resolve()?;
        match self.database.list_issues(filter) {
            Ok(issues) => {
                self.database
                    .record_successful_operation("issue_list", &context, None)?;
                Ok(issues)
            }
            Err(error) => {
                if let Err(audit_error) =
                    self.database
                        .record_failed_operation("issue_list", &context, &error)
                {
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
                self.database
                    .record_successful_operation("status", &context, None)?;
                Ok(status)
            }
            Err(error) => {
                if let Err(audit_error) = self
                    .database
                    .record_failed_operation("status", &context, &error)
                {
                    return Err(Self::failure_audit_error("status", &error, &audit_error));
                }
                Err(error)
            }
        }
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
