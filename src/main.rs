mod app;
mod cli;
mod domain;
mod error;
mod output;
mod store;

fn main() -> std::process::ExitCode {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    let output_mode = crate::output::OutputMode::from_arguments(&arguments);
    let cli = match <crate::cli::Cli as clap::Parser>::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let exit_code = error.exit_code();
            if error.print().is_err() {
                return std::process::ExitCode::from(crate::error::ExitCode::Internal as u8);
            }
            return std::process::ExitCode::from(exit_code as u8);
        }
        Err(error) => {
            crate::output::write_error(
                output_mode,
                &crate::error::AppError::InvalidInput(error.to_string()),
            );
            return std::process::ExitCode::from(crate::error::ExitCode::InvalidInput as u8);
        }
    };
    let output_mode = crate::output::OutputMode::from(&cli);

    match crate::run(cli, output_mode) {
        Ok(()) => std::process::ExitCode::from(crate::error::ExitCode::Success as u8),
        Err(error) => {
            crate::output::write_error(output_mode, &error);
            std::process::ExitCode::from(error.exit_code() as u8)
        }
    }
}

fn run(
    cli: crate::cli::Cli,
    output_mode: crate::output::OutputMode,
) -> Result<(), crate::error::AppError> {
    let resolved_context =
        crate::app::App::resolved_context(cli.project.clone(), cli.database.clone())?;
    let database_path = resolved_context.database.value.clone();
    let project = resolved_context.project.value.clone();

    match cli.command {
        crate::cli::Command::Init(_) => {
            let started_at = chrono::Utc::now();
            let execution_context = crate::domain::ExecutionContext::resolve()?;
            let _database =
                crate::store::Database::initialize(&database_path, &execution_context, started_at)?;
            match output_mode {
                crate::output::OutputMode::Human => println!("initialized"),
                crate::output::OutputMode::Json => {
                    println!(r#"{{"schema_version":1,"data":{{"initialized":true}}}}"#)
                }
            }
            Ok(())
        }
        crate::cli::Command::Project(project_command) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app = crate::app::App::new(database);

            match project_command.command {
                crate::cli::ProjectSubcommand::Create(create_command) => {
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let project = app.create_project(&create_command.name, &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            println!("{} {}", project.id, project.name)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(project),
                    }
                }
                crate::cli::ProjectSubcommand::List => {
                    let projects = app.list_projects()?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            for project in projects {
                                println!("{} {}", project.id, project.name);
                            }
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(projects),
                    }
                }
            }
            Ok(())
        }
        crate::cli::Command::Issue(issue_command) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app = crate::app::App::new(database);

            match issue_command.command {
                crate::cli::IssueSubcommand::Create(create_command) => {
                    let project = project.as_deref().ok_or_else(|| {
                        crate::error::AppError::InvalidInput("--project is required".to_owned())
                    })?;
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let issue = app.create_issue(
                        project,
                        crate::domain::NewIssue {
                            title: create_command.title,
                            body: create_command.body,
                            priority: create_command.priority,
                        },
                        &context,
                    )?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            println!("{project}#{} {}", issue.number, issue.title)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(issue),
                    }
                }
                crate::cli::IssueSubcommand::Show(show_command) => {
                    let project = project.as_deref().ok_or_else(|| {
                        crate::error::AppError::InvalidInput("--project is required".to_owned())
                    })?;
                    let issue = app.show_issue(project, show_command.number)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_issue_human(project, &issue)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(issue),
                    }
                }
                crate::cli::IssueSubcommand::List(list_command) => {
                    if list_command.all_projects && project.is_some() {
                        return Err(crate::error::AppError::InvalidInput(
                            "--all-projects cannot be used with --project".to_owned(),
                        ));
                    }
                    let projects = if list_command.all_projects {
                        Vec::new()
                    } else {
                        vec![project.clone().ok_or_else(|| {
                            crate::error::AppError::InvalidInput(
                                "--project is required unless --all-projects is used".to_owned(),
                            )
                        })?]
                    };
                    let filter = crate::domain::IssueFilter {
                        projects,
                        states: list_command.state,
                        priorities: list_command.priority,
                        assignee: list_command.assignee,
                        updated_after: list_command.updated_after,
                        query: list_command.query,
                        include_done: list_command.include_completed,
                    };
                    let issues = app.list_issues(&filter)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_issue_list_human(&issues)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(issues),
                    }
                }
                crate::cli::IssueSubcommand::Edit(command) => {
                    let project = project.as_deref().ok_or_else(|| {
                        crate::error::AppError::InvalidInput("--project is required".to_owned())
                    })?;
                    let number = command.target.number;
                    let revision = command.target.revision;
                    let patch = command.into_patch();
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let issue = app.update_issue(project, number, revision, patch, &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_issue_human(project, &issue)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(issue),
                    }
                }
                crate::cli::IssueSubcommand::Comment(command) => {
                    let project = project.as_deref().ok_or_else(|| {
                        crate::error::AppError::InvalidInput("--project is required".to_owned())
                    })?;
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let comment =
                        app.add_comment(project, command.number, &command.body, &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_comment_human(&comment)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(comment),
                    }
                }
                crate::cli::IssueSubcommand::History(command) => {
                    let project = project.as_deref().ok_or_else(|| {
                        crate::error::AppError::InvalidInput("--project is required".to_owned())
                    })?;
                    let history = app.issue_history(project, command.number)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_issue_history_human(&history)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(history),
                    }
                }
                crate::cli::IssueSubcommand::Start(command) => run_issue_transition(
                    &mut app,
                    project.as_deref(),
                    command.target,
                    "issue_start",
                    Ok(crate::domain::Transition::Start),
                    output_mode,
                )?,
                crate::cli::IssueSubcommand::Block(command) => run_issue_transition(
                    &mut app,
                    project.as_deref(),
                    command.target,
                    "issue_block",
                    crate::domain::Transition::block(command.reason, command.wait_kind),
                    output_mode,
                )?,
                crate::cli::IssueSubcommand::Resume(command) => run_issue_transition(
                    &mut app,
                    project.as_deref(),
                    command.target,
                    "issue_resume",
                    Ok(crate::domain::Transition::Resume),
                    output_mode,
                )?,
                crate::cli::IssueSubcommand::Complete(command) => run_issue_transition(
                    &mut app,
                    project.as_deref(),
                    command.target,
                    "issue_complete",
                    crate::domain::Transition::complete(command.summary, command.verification),
                    output_mode,
                )?,
                crate::cli::IssueSubcommand::Cancel(command) => run_issue_transition(
                    &mut app,
                    project.as_deref(),
                    command.target,
                    "issue_cancel",
                    crate::domain::Transition::cancel(command.reason),
                    output_mode,
                )?,
                crate::cli::IssueSubcommand::Reopen(command) => run_issue_transition(
                    &mut app,
                    project.as_deref(),
                    command.target,
                    "issue_reopen",
                    crate::domain::Transition::reopen(command.reason),
                    output_mode,
                )?,
            }
            Ok(())
        }
        crate::cli::Command::Status(_) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app = crate::app::App::new(database);
            let status = app.status()?;
            match output_mode {
                crate::output::OutputMode::Human => crate::output::write_status_human(&status),
                crate::output::OutputMode::Json => crate::output::write_success(status),
            }
            Ok(())
        }
        crate::cli::Command::Audit(audit_command) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app = crate::app::App::new(database);
            match audit_command.command {
                crate::cli::AuditSubcommand::List(command) => {
                    let events = app.list_audit_events(&crate::app::AuditFilter {
                        project_id: command.project_id,
                        operation: command.operation,
                        outcome: command.outcome,
                        kind: command.kind,
                        agent: command.agent,
                        session_id: command.session_id,
                        after: command.after,
                        before: command.before,
                    })?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_audit_events_human(&events)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(events),
                    }
                }
            }
            Ok(())
        }
        crate::cli::Command::Context(_) => {
            if database_path.is_file()
                && let Ok(database) = crate::store::Database::open(&database_path)
            {
                let mut app = crate::app::App::new(database);
                app.record_context_inspection()?;
            }
            match output_mode {
                crate::output::OutputMode::Human => {
                    crate::output::write_context_human(&resolved_context)
                }
                crate::output::OutputMode::Json => crate::output::write_success(resolved_context),
            }
            Ok(())
        }
    }
}

fn run_issue_transition(
    app: &mut crate::app::App,
    project: Option<&str>,
    target: crate::cli::IssueTransitionTarget,
    operation: &'static str,
    transition: Result<crate::domain::Transition, crate::domain::DomainError>,
    output_mode: crate::output::OutputMode,
) -> Result<(), crate::error::AppError> {
    let project = project
        .ok_or_else(|| crate::error::AppError::InvalidInput("--project is required".to_owned()))?;
    let context = crate::domain::ExecutionContext::resolve()?;
    let issue = app.transition_issue(
        project,
        target.number,
        target.revision,
        operation,
        transition,
        &context,
    )?;
    match output_mode {
        crate::output::OutputMode::Human => crate::output::write_issue_human(project, &issue),
        crate::output::OutputMode::Json => crate::output::write_success(issue),
    }
    Ok(())
}
