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
    DatabaseBusy(String),
    Internal(String),
}

impl AppError {
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::InvalidInput(_) => ExitCode::InvalidInput,
            Self::NotFound(_) => ExitCode::NotFound,
            Self::Conflict(_) => ExitCode::Conflict,
            Self::DatabaseBusy(_) => ExitCode::DatabaseBusy,
            Self::Internal(_) => ExitCode::Internal,
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::DatabaseBusy(message)
            | Self::Internal(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for AppError {}
