mod cli;
mod error;
mod output;

fn main() -> std::process::ExitCode {
    match crate::run() {
        Ok(()) => std::process::ExitCode::from(crate::error::ExitCode::Success as u8),
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::from(error.exit_code() as u8)
        }
    }
}

fn run() -> Result<(), crate::error::AppError> {
    let cli = <crate::cli::Cli as clap::Parser>::parse();
    let _output_mode = crate::output::OutputMode::from(&cli);

    match cli.command {
        crate::cli::Command::Init(_)
        | crate::cli::Command::Project(_)
        | crate::cli::Command::Issue(_)
        | crate::cli::Command::Status(_) => Err(crate::error::AppError::Internal(
            "command is not implemented".to_owned(),
        )),
    }
}
