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
    Edit(IssueEditCommand),
    Comment(IssueCommentCommand),
    History(IssueHistoryCommand),
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
pub struct IssueEditCommand {
    #[command(flatten)]
    pub target: IssueTransitionTarget,

    #[arg(long)]
    pub title: Option<String>,

    #[arg(long, conflicts_with = "clear_body")]
    pub body: Option<String>,

    #[arg(long, conflicts_with = "body")]
    pub clear_body: bool,

    #[arg(long, value_enum, conflicts_with = "clear_priority")]
    pub priority: Option<crate::domain::Priority>,

    #[arg(long, conflicts_with = "priority")]
    pub clear_priority: bool,

    #[arg(long, value_enum, conflicts_with = "clear_assignee")]
    pub assignee_kind: Option<crate::domain::AssigneeKind>,

    #[arg(long, conflicts_with = "clear_assignee")]
    pub assignee_name: Option<String>,

    #[arg(
        long,
        conflicts_with_all = ["assignee_kind", "assignee_name"]
    )]
    pub clear_assignee: bool,
}

impl IssueEditCommand {
    pub fn into_patch(self) -> crate::domain::IssuePatch {
        crate::domain::IssuePatch {
            title: self.title,
            body: if self.clear_body {
                Some(None)
            } else {
                self.body.map(Some)
            },
            priority: if self.clear_priority {
                Some(None)
            } else {
                self.priority.map(Some)
            },
            assignee_kind: if self.clear_assignee {
                Some(None)
            } else {
                self.assignee_kind.map(Some)
            },
            assignee_name: if self.clear_assignee {
                Some(None)
            } else {
                self.assignee_name.map(Some)
            },
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct IssueCommentCommand {
    pub number: i64,

    #[arg(long)]
    pub body: String,
}

#[derive(clap::Args, Debug)]
pub struct IssueHistoryCommand {
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
