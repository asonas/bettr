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

        let metadata_json = serde_json::json!({ "project_name": project.name }).to_string();
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
