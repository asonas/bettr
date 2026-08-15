#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputMode {
    Human,
    Json,
}

impl OutputMode {
    pub fn from_arguments(arguments: &[std::ffi::OsString]) -> Self {
        if arguments
            .iter()
            .any(|argument| argument.as_os_str() == std::ffi::OsStr::new("--json"))
        {
            Self::Json
        } else {
            Self::Human
        }
    }
}

impl From<&crate::cli::Cli> for OutputMode {
    fn from(cli: &crate::cli::Cli) -> Self {
        if cli.json { Self::Json } else { Self::Human }
    }
}

pub fn write_error(mode: OutputMode, error: &crate::error::AppError) {
    match mode {
        OutputMode::Human => eprintln!("{}: {error}", error.code()),
        OutputMode::Json => {
            let mut error_value = serde_json::json!({
                "schema_version": 1,
                "error": {
                    "code": error.code(),
                    "message": error.to_string(),
                }
            });
            if let crate::error::AppError::RevisionConflict { current_revision } = error {
                error_value["error"]["details"] = serde_json::json!({
                    "current_revision": current_revision,
                });
            }
            eprintln!("{error_value}");
        }
    }
}

pub fn write_success(data: impl serde::Serialize) {
    println!(
        "{}",
        serde_json::json!({ "schema_version": 1, "data": data })
    );
}

pub fn write_issue_human(project: &str, issue: &crate::domain::Issue) {
    println!("{}#{}", escape_terminal_controls(project), issue.number);
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

pub fn write_issue_list_human(issues: &[crate::domain::IssueListItem]) {
    for issue in issues {
        write_issue_summary(issue);
    }
}

pub fn write_comment_human(comment: &crate::domain::Comment) {
    println!("{}", comment.id);
    println!("{}", escape_terminal_controls(&comment.body));
}

pub fn write_issue_history_human(events: &[crate::domain::DomainEvent]) {
    for event in events {
        let revision = event
            .revision
            .map_or_else(|| "-".to_owned(), |revision| revision.to_string());
        println!(
            "{} {} {} {} {}",
            event.sequence,
            escape_terminal_controls(&event.event_type),
            revision,
            event.created_at.to_rfc3339(),
            escape_terminal_controls(&event.metadata.to_string())
        );
    }
}

pub fn write_status_human(status: &crate::domain::Status) {
    write_status_section("attention", &status.attention);
    write_status_section("stale", &status.stale);
    write_status_section("blocked", &status.blocked);
    write_status_section("recently completed", &status.recently_completed);
    write_status_section("active", &status.active);
}

pub fn write_audit_events_human(events: &[crate::app::AuditEvent]) {
    for event in events {
        let target = event.target.as_ref().map_or_else(
            || "-".to_owned(),
            |target| format!("{}:{}", target.kind, target.id),
        );
        println!(
            "{} {} {} {} {}",
            event.finished_at.to_rfc3339(),
            escape_terminal_controls(&event.operation),
            event.outcome,
            event.exit_code,
            target
        );
    }
}

pub fn write_context_human(context: &crate::app::ResolvedContext) {
    println!(
        "project: {} ({})",
        context
            .project
            .value
            .as_deref()
            .map_or_else(|| "none".to_owned(), escape_terminal_controls),
        context.project.source.as_str()
    );
    println!(
        "database: {} ({})",
        escape_terminal_controls(&context.database.value.to_string_lossy()),
        context.database.source.as_str()
    );
}

fn write_status_section(name: &str, issues: &[crate::domain::IssueListItem]) {
    if issues.is_empty() {
        return;
    }
    println!("{name}:");
    for issue in issues {
        print!("  ");
        write_issue_summary(issue);
    }
}

fn write_issue_summary(item: &crate::domain::IssueListItem) {
    println!(
        "{}#{} [{}] {}",
        escape_terminal_controls(&item.project),
        item.issue.number,
        item.issue.state.as_str(),
        escape_terminal_controls(&item.issue.title)
    );
}

fn escape_terminal_controls(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}
