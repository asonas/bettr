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
                self.database
                    .record_failed_operation("project_create", context, &error);
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
                self.database
                    .record_failed_operation("project_list", &context, &error);
                Err(error)
            }
        }
    }
}
