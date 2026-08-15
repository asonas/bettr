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
