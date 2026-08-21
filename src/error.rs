#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    InvalidInput = 2,
    NotFound = 3,
    Conflict = 4,
    DatabaseBusy = 5,
    Internal = 10,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum AppError {
    InvalidInput(String),
    NotFound(String),
    Conflict(String),
    IdempotencyConflict,
    InvalidTransition(String),
    RevisionConflict {
        current_revision: i64,
    },
    ProjectNameConflict,
    DatabaseBusy(String),
    Internal(String),
    AuditIntegrity {
        message: String,
        line: Option<usize>,
        sequence: Option<i64>,
    },
    AuditOperation {
        operation: &'static str,
    },
    DatabaseAlreadyInitialized,
    DatabaseNotInitialized,
    UnsupportedDatabaseSchemaVersion {
        found_version: u32,
        current_version: u32,
    },
    SelfUpdateFailed(serde_json::Value),
    SelfUninstallFailed(serde_json::Value),
    InvalidBackup(String),
    BackupOutputExists,
    BackupConfirmationRequired,
    BackupDestinationInUse,
    BackupOperation {
        operation: &'static str,
    },
}

impl AppError {
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::InvalidInput(_) => ExitCode::InvalidInput,
            Self::NotFound(_) => ExitCode::NotFound,
            Self::Conflict(_)
            | Self::IdempotencyConflict
            | Self::InvalidTransition(_)
            | Self::RevisionConflict { .. } => ExitCode::Conflict,
            Self::ProjectNameConflict => ExitCode::Conflict,
            Self::DatabaseBusy(_) => ExitCode::DatabaseBusy,
            Self::Internal(_) | Self::AuditIntegrity { .. } | Self::AuditOperation { .. } => {
                ExitCode::Internal
            }
            Self::DatabaseAlreadyInitialized => ExitCode::InvalidInput,
            Self::DatabaseNotInitialized => ExitCode::NotFound,
            Self::UnsupportedDatabaseSchemaVersion { .. } => ExitCode::InvalidInput,
            Self::SelfUpdateFailed(_) | Self::SelfUninstallFailed(_) => ExitCode::Internal,
            Self::InvalidBackup(_) | Self::BackupConfirmationRequired => ExitCode::InvalidInput,
            Self::BackupOutputExists | Self::BackupDestinationInUse => ExitCode::Conflict,
            Self::BackupOperation { .. } => ExitCode::Internal,
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::InvalidTransition(_) => "invalid_transition",
            Self::RevisionConflict { .. } => "revision_conflict",
            Self::ProjectNameConflict => "project_name_conflict",
            Self::DatabaseBusy(_) => "database_busy",
            Self::Internal(_) => "internal_error",
            Self::AuditIntegrity { .. } => "audit_integrity_failure",
            Self::AuditOperation { .. } => "audit_operation_failed",
            Self::DatabaseAlreadyInitialized => "database_already_initialized",
            Self::DatabaseNotInitialized => "database_not_initialized",
            Self::UnsupportedDatabaseSchemaVersion { .. } => "unsupported_database_schema_version",
            Self::SelfUpdateFailed(_) => "self_update_failed",
            Self::SelfUninstallFailed(_) => "self_uninstall_failed",
            Self::InvalidBackup(_) => "invalid_backup",
            Self::BackupOutputExists => "backup_output_exists",
            Self::BackupConfirmationRequired => "confirmation_required",
            Self::BackupDestinationInUse => "backup_destination_in_use",
            Self::BackupOperation { .. } => "backup_operation_failed",
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::InvalidTransition(message)
            | Self::DatabaseBusy(message)
            | Self::Internal(message) => formatter.write_str(message),
            Self::AuditIntegrity { message, .. } => formatter.write_str(message),
            Self::AuditOperation { operation } => {
                write!(formatter, "audit {operation} operation failed")
            }
            Self::IdempotencyConflict => formatter.write_str(
                "idempotency key has already been used with a different operation or payload",
            ),
            Self::RevisionConflict { current_revision } => write!(
                formatter,
                "issue revision conflict; current revision is {current_revision}"
            ),
            Self::ProjectNameConflict => formatter.write_str("project name already exists"),
            Self::DatabaseAlreadyInitialized => {
                formatter.write_str("database is already initialized")
            }
            Self::DatabaseNotInitialized => formatter.write_str("database is not initialized"),
            Self::UnsupportedDatabaseSchemaVersion {
                found_version,
                current_version,
            } => write!(
                formatter,
                "database schema version {found_version} is unsupported; current version is {current_version}"
            ),
            Self::SelfUpdateFailed(_) => formatter.write_str("self-update failed"),
            Self::SelfUninstallFailed(_) => formatter.write_str("self-uninstall failed"),
            Self::InvalidBackup(message) => formatter.write_str(message),
            Self::BackupOutputExists => formatter.write_str("backup output already exists"),
            Self::BackupConfirmationRequired => {
                formatter.write_str("restore confirmation is required")
            }
            Self::BackupDestinationInUse => {
                formatter.write_str("destination has SQLite sidecar files")
            }
            Self::BackupOperation { operation } => {
                write!(formatter, "backup {operation} operation failed")
            }
        }
    }
}

impl std::error::Error for AppError {}

impl From<crate::domain::DomainError> for AppError {
    fn from(error: crate::domain::DomainError) -> Self {
        match error {
            crate::domain::DomainError::InvalidMetadata(message) => Self::InvalidInput(message),
            crate::domain::DomainError::InvalidTransition { .. } => {
                Self::InvalidTransition(error.to_string())
            }
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(code, _)
                if matches!(
                    code.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                Self::DatabaseBusy("database is busy".to_owned())
            }
            rusqlite::Error::SqliteFailure(code, _) => Self::Internal(format!(
                "database operation failed (SQLite error code {})",
                code.extended_code
            )),
            _ => Self::Internal("database operation failed".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    fn sqlite_failure(code: i32, message: &str) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), Some(message.to_owned()))
    }

    #[test]
    fn sqlite_busy_variants_and_locked_map_to_database_busy() {
        for code in [
            rusqlite::ffi::SQLITE_BUSY,
            rusqlite::ffi::SQLITE_BUSY_SNAPSHOT,
            rusqlite::ffi::SQLITE_LOCKED,
        ] {
            let error = super::AppError::from(sqlite_failure(code, "private SQLite detail"));
            assert!(matches!(error, super::AppError::DatabaseBusy(_)));
            assert!(!error.to_string().contains("private SQLite detail"));
        }
    }

    #[test]
    fn other_sqlite_errors_have_safe_internal_diagnostics() {
        let error = super::AppError::from(sqlite_failure(
            rusqlite::ffi::SQLITE_CORRUPT,
            "secret database detail leaked by SQLite",
        ));

        assert!(matches!(error, super::AppError::Internal(_)));
        assert!(!error.to_string().contains("secret database detail"));
        assert!(error.to_string().contains("SQLite error code 11"));
    }

    #[test]
    fn unsupported_schema_version_has_a_stable_input_error_contract() {
        let error = super::AppError::UnsupportedDatabaseSchemaVersion {
            found_version: 99,
            current_version: 3,
        };

        assert_eq!(error.exit_code() as u8, 2);
        assert_eq!(error.code(), "unsupported_database_schema_version");
        assert_eq!(
            error.to_string(),
            "database schema version 99 is unsupported; current version is 3"
        );
    }

    #[test]
    fn idempotency_conflict_has_a_stable_contract() {
        let error = super::AppError::IdempotencyConflict;

        assert_eq!(error.exit_code() as u8, 4);
        assert_eq!(error.code(), "idempotency_conflict");
    }
}
