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
    List(IssueListCommand),
    Start(IssueStartCommand),
    Block(IssueBlockCommand),
    Resume(IssueResumeCommand),
    Complete(IssueCompleteCommand),
    Cancel(IssueCancelCommand),
    Reopen(IssueReopenCommand),
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
pub struct IssueListCommand {
    #[arg(long)]
    pub all_projects: bool,

    #[arg(long, value_enum)]
    pub state: Vec<crate::domain::IssueState>,

    #[arg(long, value_enum)]
    pub priority: Vec<crate::domain::Priority>,

    #[arg(long)]
    pub assignee: Option<String>,

    #[arg(long, value_parser = parse_timestamp)]
    pub updated_after: Option<chrono::DateTime<chrono::Utc>>,

    #[arg(long)]
    pub query: Option<String>,

    #[arg(long)]
    pub include_completed: bool,
}

#[derive(clap::Args, Debug)]
pub struct IssueTransitionTarget {
    pub number: i64,

    #[arg(long)]
    pub revision: i64,
}

#[derive(clap::Args, Debug)]
pub struct IssueStartCommand {
    #[command(flatten)]
    pub target: IssueTransitionTarget,
}

#[derive(clap::Args, Debug)]
pub struct IssueBlockCommand {
    #[command(flatten)]
    pub target: IssueTransitionTarget,

    #[arg(long)]
    pub reason: String,

    #[arg(long, value_enum)]
    pub wait_kind: crate::domain::WaitKind,
}

#[derive(clap::Args, Debug)]
pub struct IssueResumeCommand {
    #[command(flatten)]
    pub target: IssueTransitionTarget,
}

#[derive(clap::Args, Debug)]
pub struct IssueCompleteCommand {
    #[command(flatten)]
    pub target: IssueTransitionTarget,

    #[arg(long)]
    pub summary: String,

    #[arg(long)]
    pub verification: String,
}

#[derive(clap::Args, Debug)]
pub struct IssueCancelCommand {
    #[command(flatten)]
    pub target: IssueTransitionTarget,

    #[arg(long)]
    pub reason: String,
}

#[derive(clap::Args, Debug)]
pub struct IssueReopenCommand {
    #[command(flatten)]
    pub target: IssueTransitionTarget,

    #[arg(long)]
    pub reason: String,
}

#[derive(clap::Args, Debug)]
pub struct StatusCommand {}

fn parse_timestamp(value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    value
        .parse::<chrono::DateTime<chrono::Utc>>()
        .map_err(|_| "must be an RFC 3339 timestamp".to_owned())
}
