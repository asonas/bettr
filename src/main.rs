mod app;
mod cli;
mod domain;
mod error;
mod output;
mod self_update;
mod store;
mod web;

fn main() -> std::process::ExitCode {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    let started_at = chrono::Utc::now();
    let output_mode = crate::output::OutputMode::from_arguments(&arguments);
    let cli = match <crate::cli::Cli as clap::Parser>::try_parse_from(arguments.clone()) {
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
            let error = crate::audit_unparsed_cli_failure(
                &arguments,
                crate::error::AppError::InvalidInput(error.to_string()),
                started_at,
            );
            crate::output::write_error(output_mode, &error);
            return std::process::ExitCode::from(error.exit_code() as u8);
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
    if let crate::cli::Command::SelfUpdate(command) = &cli.command {
        let source = crate::app::App::resolved_update_source(command.source)?.value;
        let report = crate::self_update::run(source)?;
        if output_mode == crate::output::OutputMode::Human {
            crate::output::write_self_update_human(&report);
        }
        if report.succeeded() {
            if output_mode == crate::output::OutputMode::Json {
                crate::output::write_success(report);
            }
            return Ok(());
        }
        let report = serde_json::to_value(report).map_err(|error| {
            crate::error::AppError::Internal(format!(
                "could not serialize self-update result: {error}"
            ))
        })?;
        return Err(crate::error::AppError::SelfUpdateFailed(report));
    }

    let resolved_context =
        crate::app::App::resolved_context(cli.project.clone(), cli.database.clone())?;
    let database_path = resolved_context.database.value.clone();
    let project = resolved_context.project.value.clone();
    let idempotency_key = cli.idempotency_key.clone();

    match cli.command {
        crate::cli::Command::Init(_) => {
            let started_at = chrono::Utc::now();
            let execution_context = crate::domain::ExecutionContext::resolve()?;
            let _database = crate::store::Database::initialize_with_idempotency(
                &database_path,
                &execution_context,
                started_at,
                idempotency_key.as_deref(),
            )?;
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
            let mut app =
                crate::app::App::new(database).with_idempotency_key(idempotency_key.clone())?;

            match project_command.command {
                crate::cli::ProjectSubcommand::Create(create_command) => {
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let project = app.create_project(&create_command.name, &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_project_human(&project)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(project),
                    }
                }
                crate::cli::ProjectSubcommand::List => {
                    let projects = app.list_projects()?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_projects_human(&projects)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(projects),
                    }
                }
            }
            Ok(())
        }
        crate::cli::Command::Issue(issue_command) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app =
                crate::app::App::new(database).with_idempotency_key(idempotency_key.clone())?;

            match issue_command.command {
                crate::cli::IssueSubcommand::Create(create_command) => {
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "issue_create")?;
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
                            crate::output::write_issue_created_human(project, &issue)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(issue),
                    }
                }
                crate::cli::IssueSubcommand::Batch(batch_command) => {
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let results =
                        app.batch_issues(&batch_command.input, project.as_deref(), &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_batch_human(&results)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(results),
                    }
                }
                crate::cli::IssueSubcommand::Show(show_command) => {
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "issue_show")?;
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
                        return Err(crate::audited_invalid_input(
                            &mut app,
                            "issue_list",
                            project.as_deref(),
                            "--all-projects cannot be used with --project",
                        ));
                    }
                    let projects = if list_command.all_projects {
                        Vec::new()
                    } else {
                        vec![
                            crate::require_project(&mut app, project.as_deref(), "issue_list")?
                                .to_owned(),
                        ]
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
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "issue_edit")?;
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
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "issue_comment")?;
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
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "issue_history")?;
                    let history = app.issue_history(project, command.number)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_issue_history_human(&history)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(history),
                    }
                }
                crate::cli::IssueSubcommand::Dependency(command) => match command.command {
                    crate::cli::IssueDependencySubcommand::Add(command) => {
                        let context = crate::domain::ExecutionContext::resolve()?;
                        let relation = app.add_dependency(
                            &command.blocker,
                            &command.blocked,
                            project.as_deref(),
                            &context,
                        )?;
                        match output_mode {
                            crate::output::OutputMode::Human => {
                                crate::output::write_issue_dependency_human(&relation)
                            }
                            crate::output::OutputMode::Json => {
                                crate::output::write_success(relation)
                            }
                        }
                    }
                    crate::cli::IssueDependencySubcommand::Remove(command) => {
                        let context = crate::domain::ExecutionContext::resolve()?;
                        let relation = app.remove_dependency(
                            &command.blocker,
                            &command.blocked,
                            project.as_deref(),
                            &context,
                        )?;
                        match output_mode {
                            crate::output::OutputMode::Human => {
                                crate::output::write_issue_dependency_human(&relation)
                            }
                            crate::output::OutputMode::Json => {
                                crate::output::write_success(relation)
                            }
                        }
                    }
                    crate::cli::IssueDependencySubcommand::List(command) => {
                        let relations =
                            app.list_dependencies(&command.reference, project.as_deref())?;
                        match output_mode {
                            crate::output::OutputMode::Human => {
                                crate::output::write_issue_dependencies_human(&relations)
                            }
                            crate::output::OutputMode::Json => {
                                crate::output::write_success(relations)
                            }
                        }
                    }
                },
                crate::cli::IssueSubcommand::Parent(command) => match command.command {
                    crate::cli::IssueParentSubcommand::Set(command) => {
                        let context = crate::domain::ExecutionContext::resolve()?;
                        let relation = app.set_parent(
                            &command.child,
                            &command.parent,
                            project.as_deref(),
                            &context,
                        )?;
                        match output_mode {
                            crate::output::OutputMode::Human => {
                                crate::output::write_issue_parent_human(&relation)
                            }
                            crate::output::OutputMode::Json => {
                                crate::output::write_success(relation)
                            }
                        }
                    }
                    crate::cli::IssueParentSubcommand::List(command) => {
                        let relations = app.list_parent(&command.reference, project.as_deref())?;
                        match output_mode {
                            crate::output::OutputMode::Human => {
                                crate::output::write_issue_parents_human(&relations)
                            }
                            crate::output::OutputMode::Json => {
                                crate::output::write_success(relations)
                            }
                        }
                    }
                },
                crate::cli::IssueSubcommand::Claim(command) => {
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let claimed = app.claim_issue(project.as_deref(), command.number, &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_claimed_issue_human(&claimed)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(claimed),
                    }
                }
                crate::cli::IssueSubcommand::Heartbeat(command) => {
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "issue_heartbeat")?;
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let claimed = app.heartbeat_issue(project, command.number, &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_claimed_issue_human(&claimed)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(claimed),
                    }
                }
                crate::cli::IssueSubcommand::Takeover(command) => {
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "issue_takeover")?;
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let claimed =
                        app.takeover_issue(project, command.number, &command.reason, &context)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_claimed_issue_human(&claimed)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(claimed),
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
        crate::cli::Command::Decision(decision_command) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app =
                crate::app::App::new(database).with_idempotency_key(idempotency_key.clone())?;
            match decision_command.command {
                crate::cli::DecisionSubcommand::Request(command) => {
                    let project =
                        crate::require_project(&mut app, project.as_deref(), "decision_request")?;
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let request = app.request_decision(
                        project,
                        command.number,
                        &command.question,
                        &command.background,
                        &context,
                    )?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_decision_human(&request)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(request),
                    }
                }
                crate::cli::DecisionSubcommand::Resolve(command) => {
                    let context = crate::domain::ExecutionContext::resolve()?;
                    let resolution = crate::domain::DecisionResolutionInput::new(
                        command.next_state,
                        command.summary,
                        command.verification,
                        command.reason,
                        command.wait_kind,
                    );
                    let request = app.resolve_decision(
                        &command.request_id,
                        &command.answer,
                        None,
                        resolution,
                        &context,
                    )?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_decision_human(&request)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(request),
                    }
                }
            }
            Ok(())
        }
        crate::cli::Command::Event(event_command) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app =
                crate::app::App::new(database).with_idempotency_key(idempotency_key.clone())?;
            match event_command.command {
                crate::cli::EventSubcommand::List(command) => {
                    let page =
                        app.list_events(command.after, command.limit, command.include_issue)?;
                    match output_mode {
                        crate::output::OutputMode::Human => {
                            crate::output::write_event_page_human(&page)
                        }
                        crate::output::OutputMode::Json => crate::output::write_success(page),
                    }
                }
            }
            Ok(())
        }
        crate::cli::Command::Capabilities(_) => {
            let capabilities = crate::app::App::capabilities();
            match output_mode {
                crate::output::OutputMode::Human => {
                    crate::output::write_capabilities_human(&capabilities)
                }
                crate::output::OutputMode::Json => crate::output::write_success(capabilities),
            }
            Ok(())
        }
        crate::cli::Command::Status(_) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app =
                crate::app::App::new(database).with_idempotency_key(idempotency_key.clone())?;
            let status = app.status()?;
            match output_mode {
                crate::output::OutputMode::Human => crate::output::write_status_human(&status),
                crate::output::OutputMode::Json => crate::output::write_success(status),
            }
            Ok(())
        }
        crate::cli::Command::Audit(audit_command) => {
            let database = crate::store::Database::open(&database_path)?;
            let mut app =
                crate::app::App::new(database).with_idempotency_key(idempotency_key.clone())?;
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
                let mut app =
                    crate::app::App::new(database).with_idempotency_key(idempotency_key.clone())?;
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
        crate::cli::Command::SelfUpdate(_) => {
            unreachable!("self-update is handled before DB resolution")
        }
        crate::cli::Command::Web(web_command) => crate::web::run(&database_path, web_command.port),
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
    let project = crate::require_project(app, project, operation)?;
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

fn require_project<'a>(
    app: &mut crate::app::App,
    project: Option<&'a str>,
    operation: &str,
) -> Result<&'a str, crate::error::AppError> {
    project
        .ok_or_else(|| crate::audited_invalid_input(app, operation, None, "--project is required"))
}

fn audited_invalid_input(
    app: &mut crate::app::App,
    operation: &str,
    project: Option<&str>,
    message: &str,
) -> crate::error::AppError {
    let error = crate::error::AppError::InvalidInput(message.to_owned());
    let Ok(context) = crate::domain::ExecutionContext::resolve() else {
        return error;
    };
    app.audited_cli_failure(operation, project, &context, error, chrono::Utc::now())
}

fn audit_unparsed_cli_failure(
    arguments: &[std::ffi::OsString],
    error: crate::error::AppError,
    started_at: chrono::DateTime<chrono::Utc>,
) -> crate::error::AppError {
    let Some(invocation) = crate::cli::AuditInvocation::from_arguments(arguments) else {
        return error;
    };
    let Ok(resolved) = crate::app::App::resolved_context(invocation.project, invocation.database)
    else {
        return error;
    };
    let Ok(context) = crate::domain::ExecutionContext::resolve() else {
        return error;
    };
    let Ok(database) = crate::store::Database::open(&resolved.database.value) else {
        return error;
    };
    let mut app = crate::app::App::new(database);
    app.audited_cli_failure(
        invocation.operation,
        resolved.project.value.as_deref(),
        &context,
        error,
        started_at,
    )
}
