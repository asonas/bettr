#[derive(clap::Parser, Debug)]
#[command(name = "bettr", about = "Local issue tracking for agent work")]
pub struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    pub database: Option<std::path::PathBuf>,

    #[arg(long, global = true)]
    pub project: Option<String>,

    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    Init(InitCommand),
    Project(ProjectCommand),
    Issue(IssueCommand),
    Status(StatusCommand),
}

#[derive(clap::Args, Debug)]
pub struct InitCommand {}

#[derive(clap::Args, Debug)]
pub struct ProjectCommand {
    #[command(subcommand)]
    pub command: ProjectSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum ProjectSubcommand {
    Create(ProjectCreateCommand),
    List,
}

#[derive(clap::Args, Debug)]
pub struct ProjectCreateCommand {
    pub name: String,
}

#[derive(clap::Args, Debug)]
pub struct IssueCommand {
    #[command(subcommand)]
    pub command: IssueSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum IssueSubcommand {
    Create(IssueCreateCommand),
    Show(IssueShowCommand),
    List,
}

#[derive(clap::Args, Debug)]
pub struct IssueCreateCommand {
    #[arg(long)]
    pub title: String,

    #[arg(long)]
    pub body: Option<String>,

    #[arg(long, value_enum)]
    pub priority: Option<crate::domain::Priority>,
}

#[derive(clap::Args, Debug)]
pub struct IssueShowCommand {
    pub number: i64,
}

#[derive(clap::Args, Debug)]
pub struct StatusCommand {}
