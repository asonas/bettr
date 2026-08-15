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
