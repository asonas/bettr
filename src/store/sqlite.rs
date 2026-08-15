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
                let _ = std::fs::remove_file(path);
                return Err(error);
            }
        };

        if let Err(error) = database.initialize_schema() {
            drop(database);
            let _ = std::fs::remove_file(path);
            return Err(error);
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
}
