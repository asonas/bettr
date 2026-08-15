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

pub fn write_success(data: impl serde::Serialize) {
    println!(
        "{}",
        serde_json::json!({ "schema_version": 1, "data": data })
    );
}

pub fn write_issue_human(project: &str, issue: &crate::domain::Issue) {
    println!("{project}#{}", issue.number);
    println!("state: {}", issue.state.as_str());
    println!("title: {}", escape_terminal_controls(&issue.title));
    println!("revision: {}", issue.revision);
    println!(
        "priority: {}",
        issue
            .priority
            .map_or("none", crate::domain::Priority::as_str)
    );
    println!(
        "assignee: {}",
        issue
            .assignee_name
            .as_deref()
            .map_or_else(|| "none".to_owned(), escape_terminal_controls)
    );
    println!("body:");
    println!(
        "{}",
        issue
            .body
            .as_deref()
            .map_or_else(|| "none".to_owned(), escape_terminal_controls)
    );
}

fn escape_terminal_controls(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}
