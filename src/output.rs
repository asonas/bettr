#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputMode {
    Human,
    Json,
}

impl From<&crate::cli::Cli> for OutputMode {
    fn from(cli: &crate::cli::Cli) -> Self {
        if cli.json { Self::Json } else { Self::Human }
    }
}

pub fn write_error(mode: OutputMode, error: &crate::error::AppError) {
    match mode {
        OutputMode::Human => eprintln!("{}: {error}", error.code()),
        OutputMode::Json => eprintln!(
            "{}",
            serde_json::json!({
                "schema_version": 1,
                "error": {
                    "code": error.code(),
                    "message": error.to_string(),
                }
            })
        ),
    }
}
