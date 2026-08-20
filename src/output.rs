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
            match error {
                crate::error::AppError::RevisionConflict { current_revision } => {
                    error_value["error"]["details"] = serde_json::json!({
                        "current_revision": current_revision,
                    });
                }
                crate::error::AppError::UnsupportedDatabaseSchemaVersion {
                    found_version,
                    current_version,
                } => {
                    error_value["error"]["details"] = serde_json::json!({
                        "found_version": found_version,
                        "current_version": current_version,
                    });
                }
                crate::error::AppError::SelfUpdateFailed(report) => {
                    error_value["error"]["details"] = report.clone();
                }
                crate::error::AppError::AuditIntegrity { line, sequence, .. } => {
                    error_value["error"]["details"] = serde_json::json!({
                        "line": line,
                        "sequence": sequence,
                        "recovery":
                            "preserve the affected JSONL and run bettr audit rebuild --json",
                    });
                }
                crate::error::AppError::AuditOperation { operation } => {
                    error_value["error"]["details"] = serde_json::json!({
                        "operation": operation,
                        "recovery":
                            "preserve the affected JSONL and retry, or run bettr audit rebuild --json",
                    });
                }
                _ => {}
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

pub fn write_self_update_human(report: &crate::self_update::SelfUpdateReport) {
    println!("source: {}", report.source.as_str());
    println!("revision: {}", escape_terminal_controls(&report.revision));
    write_self_update_component("cli", &report.cli);
    write_self_update_component("codex", &report.codex);
    write_self_update_component("claude", &report.claude);
}

fn write_self_update_component(name: &str, component: &crate::self_update::ComponentUpdate) {
    let version = component
        .version
        .as_deref()
        .map_or_else(|| "-".to_owned(), escape_terminal_controls);
    println!(
        "{name}: {} source={} version={} revision={}",
        component.result.as_str(),
        component.source.as_str(),
        version,
        escape_terminal_controls(&component.revision)
    );
    println!("  path: {}", escape_terminal_controls(&component.path));
    if let Some(backup) = &component.backup {
        println!("  backup: {}", escape_terminal_controls(backup));
    }
    if let Some(error) = &component.error {
        println!("  error: {}", escape_terminal_controls(error));
    }
}

pub fn write_project_human(project: &crate::domain::Project) {
    println!("{} {}", project.id, escape_terminal_controls(&project.name));
}

pub fn write_projects_human(projects: &[crate::domain::Project]) {
    for project in projects {
        write_project_human(project);
    }
}

pub fn write_backup_human(result: &crate::store::backup::BackupResult) {
    println!(
        "backup {}",
        escape_terminal_controls(&result.output.display().to_string())
    );
}

pub fn write_restore_human(result: &crate::store::backup::RestoreResult) {
    println!(
        "restored {}",
        escape_terminal_controls(&result.output.display().to_string())
    );
}

pub fn write_issue_created_human(project: &str, issue: &crate::domain::Issue) {
    println!(
        "{}#{} {}",
        escape_terminal_controls(project),
        issue.number,
        escape_terminal_controls(&issue.title)
    );
}

pub fn write_batch_human(results: &[crate::domain::BatchResult]) {
    for result in results {
        println!(
            "{} {}",
            result.operation,
            escape_terminal_controls(&result.result.to_string())
        );
    }
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

pub fn write_issue_details_human(project: &str, details: &crate::domain::IssueDetails) {
    write_issue_human(project, &details.issue);
    println!("dependencies:");
    if details.dependencies.is_empty() {
        println!("none");
    } else {
        write_issue_dependencies_human(&details.dependencies);
    }
    println!("worktrees:");
    if details.worktrees.is_empty() {
        println!("none");
    } else {
        write_issue_worktrees_human(&details.worktrees);
    }
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

pub fn write_redaction_human(result: &crate::domain::RedactionResult) {
    println!(
        "redacted {} {} ({} records)",
        escape_terminal_controls(&result.target_type),
        result.target_id,
        result.changed_count
    );
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
            event.created_at.with_timezone(&chrono::Local).to_rfc3339(),
            escape_terminal_controls(&event.metadata.to_string())
        );
    }
}

pub fn write_issue_dependency_human(relation: &crate::domain::IssueDependency) {
    println!(
        "{} {} {}",
        escape_terminal_controls(&relation.blocker),
        relation.relation,
        escape_terminal_controls(&relation.blocked)
    );
}

pub fn write_issue_dependencies_human(relations: &[crate::domain::IssueDependency]) {
    for relation in relations {
        write_issue_dependency_human(relation);
    }
}

pub fn write_issue_worktree_human(worktree: &crate::domain::IssueWorktree) {
    let branch = worktree.branch.as_deref().unwrap_or("detached");
    let state = if worktree.active {
        "active"
    } else {
        "inactive"
    };
    println!(
        "{} {} branch={} path={}",
        escape_terminal_controls(&worktree.issue),
        state,
        escape_terminal_controls(branch),
        escape_terminal_controls(&worktree.path),
    );
}

pub fn write_issue_worktrees_human(worktrees: &[crate::domain::IssueWorktree]) {
    for worktree in worktrees {
        write_issue_worktree_human(worktree);
    }
}

pub fn write_issue_parent_human(relation: &crate::domain::IssueParent) {
    println!(
        "{} parent {}",
        escape_terminal_controls(&relation.child),
        escape_terminal_controls(&relation.parent)
    );
}

pub fn write_issue_parents_human(relations: &[crate::domain::IssueParent]) {
    for relation in relations {
        write_issue_parent_human(relation);
    }
}

pub fn write_claimed_issue_human(claimed: &crate::domain::ClaimedIssue) {
    println!(
        "#{} [{}] {}",
        claimed.issue.number,
        claimed.issue.state.as_str(),
        escape_terminal_controls(&claimed.issue.title)
    );
    println!(
        "lease: {}",
        escape_terminal_controls(&claimed.lease.session_id)
    );
    println!("expires_at: {}", claimed.lease.expires_at.to_rfc3339());
}

pub fn write_decision_human(request: &crate::domain::DecisionRequest) {
    println!(
        "{} {}",
        request.id,
        escape_terminal_controls(&request.issue)
    );
    println!("status: {}", request.status);
    println!("blocker: {}", escape_terminal_controls(&request.blocker));
    println!("question: {}", escape_terminal_controls(&request.question));
    println!("options:");
    for option in &request.options {
        println!("- {}", escape_terminal_controls(option));
    }
    println!(
        "recommendation: {}",
        escape_terminal_controls(&request.recommendation)
    );
    println!(
        "resume_condition: {}",
        escape_terminal_controls(&request.resume_condition)
    );
    println!(
        "background: {}",
        escape_terminal_controls(&request.background)
    );
    if let Some(answer) = &request.answer {
        println!("answer: {}", escape_terminal_controls(answer));
    }
}

pub fn write_capabilities_human(capabilities: &crate::domain::CapabilitySet) {
    println!(
        "json_contract_version: {}",
        capabilities.json_contract_version
    );
    println!("cli_version: {}", capabilities.cli_version);
    for (name, available) in &capabilities.capabilities {
        println!("{name}: {available}");
    }
}

pub fn write_event_page_human(page: &crate::domain::EventPage) {
    for event in &page.events {
        println!(
            "{} {} {}",
            event.sequence,
            escape_terminal_controls(&event.event_type),
            event.created_at.with_timezone(&chrono::Local).to_rfc3339()
        );
    }
    println!("next_cursor: {}", page.next_cursor);
    println!("has_more: {}", page.has_more);
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
            event.finished_at.with_timezone(&chrono::Local).to_rfc3339(),
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
    let worktrees = item
        .worktrees
        .iter()
        .map(|worktree| worktree.branch.as_deref().unwrap_or("detached"))
        .collect::<Vec<_>>();
    let suffix = if worktrees.is_empty() {
        String::new()
    } else {
        format!(
            " worktrees={}",
            escape_terminal_controls(&worktrees.join(","))
        )
    };
    println!(
        "{}#{} [{}] {}{}",
        escape_terminal_controls(&item.project),
        item.issue.number,
        item.issue.state.as_str(),
        escape_terminal_controls(&item.issue.title),
        suffix,
    );
}

fn escape_terminal_controls(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}
