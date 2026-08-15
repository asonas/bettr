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
    let database_path = cli
        .database
        .as_deref()
        .ok_or_else(|| crate::error::AppError::InvalidInput("--database is required".to_owned()))?;

    match cli.command {
        crate::cli::Command::Init(_) => {
            let _database = crate::store::Database::initialize(database_path)?;
            match output_mode {
                crate::output::OutputMode::Human => println!("initialized"),
                crate::output::OutputMode::Json => {
                    println!(r#"{{"schema_version":1,"data":{{"initialized":true}}}}"#)
                }
            }
            Ok(())
        }
        crate::cli::Command::Project(project_command) => {
            let database = crate::store::Database::open(database_path)?;
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
            let database = crate::store::Database::open(database_path)?;
            let mut app = crate::app::App::new(database);

            match issue_command.command {
                crate::cli::IssueSubcommand::Create(create_command) => {
                    let project = cli.project.as_deref().ok_or_else(|| {
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
                    let project = cli.project.as_deref().ok_or_else(|| {
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
                crate::cli::IssueSubcommand::List => {
                    return Err(crate::error::AppError::Internal(
                        "command is not implemented".to_owned(),
                    ));
                }
            }
            Ok(())
        }
        crate::cli::Command::Status(_) => {
            let _database = crate::store::Database::open(database_path)?;
            Err(crate::error::AppError::Internal(
                "command is not implemented".to_owned(),
            ))
        }
    }
}
