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
    InvalidTransition(String),
    RevisionConflict { current_revision: i64 },
    ProjectNameConflict,
    DatabaseBusy(String),
    Internal(String),
    DatabaseAlreadyInitialized,
    DatabaseNotInitialized,
}

impl AppError {
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::InvalidInput(_) => ExitCode::InvalidInput,
            Self::NotFound(_) => ExitCode::NotFound,
            Self::Conflict(_) | Self::InvalidTransition(_) | Self::RevisionConflict { .. } => {
                ExitCode::Conflict
            }
            Self::ProjectNameConflict => ExitCode::Conflict,
            Self::DatabaseBusy(_) => ExitCode::DatabaseBusy,
            Self::Internal(_) => ExitCode::Internal,
            Self::DatabaseAlreadyInitialized => ExitCode::InvalidInput,
            Self::DatabaseNotInitialized => ExitCode::NotFound,
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::InvalidTransition(_) => "invalid_transition",
            Self::RevisionConflict { .. } => "revision_conflict",
            Self::ProjectNameConflict => "project_name_conflict",
            Self::DatabaseBusy(_) => "database_busy",
            Self::Internal(_) => "internal_error",
            Self::DatabaseAlreadyInitialized => "database_already_initialized",
            Self::DatabaseNotInitialized => "database_not_initialized",
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
            Self::RevisionConflict { current_revision } => write!(
                formatter,
                "issue revision conflict; current revision is {current_revision}"
            ),
            Self::ProjectNameConflict => formatter.write_str("project name already exists"),
            Self::DatabaseAlreadyInitialized => {
                formatter.write_str("database is already initialized")
            }
            Self::DatabaseNotInitialized => formatter.write_str("database is not initialized"),
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
