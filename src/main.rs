mod cli;
mod error;
mod output;
mod store;

fn main() -> std::process::ExitCode {
    let cli = <crate::cli::Cli as clap::Parser>::parse();
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
        crate::cli::Command::Project(_)
        | crate::cli::Command::Issue(_)
        | crate::cli::Command::Status(_) => {
            let _database = crate::store::Database::open(database_path)?;
            Err(crate::error::AppError::Internal(
                "command is not implemented".to_owned(),
            ))
        }
    }
}
