#[derive(clap::Parser, Debug)]
#[command(name = "bettr", about = "Local issue tracking for agent work", version)]
pub struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    pub database: Option<std::path::PathBuf>,

    #[arg(long, global = true)]
    pub project: Option<String>,

    #[arg(long, global = true)]
    pub json: bool,

    #[arg(long, global = true, value_name = "KEY")]
    pub idempotency_key: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

pub struct AuditInvocation {
    pub operation: &'static str,
    pub database: Option<std::path::PathBuf>,
    pub project: Option<String>,
}

impl AuditInvocation {
    pub fn from_arguments(arguments: &[std::ffi::OsString]) -> Option<Self> {
        let mut database = None;
        let mut project = None;
        let mut positionals = Vec::new();
        let mut index = 1;
        while index < arguments.len() {
            let Some(argument) = arguments[index].to_str() else {
                index += 1;
                continue;
            };
            match argument {
                "--database" => {
                    index += 1;
                    database = arguments.get(index).map(std::path::PathBuf::from);
                }
                "--project" => {
                    index += 1;
                    project = arguments
                        .get(index)
                        .and_then(|value| value.to_str())
                        .map(str::to_owned);
                }
                "--idempotency-key" => {
                    index += 1;
                }
                _ if argument.starts_with("--database=") => {
                    database = argument
                        .strip_prefix("--database=")
                        .map(std::path::PathBuf::from);
                }
                _ if argument.starts_with("--project=") => {
                    project = argument.strip_prefix("--project=").map(str::to_owned);
                }
                _ if argument.starts_with('-') => {}
                _ => positionals.push(argument),
            }
            index += 1;
        }

        let root = positionals.first().copied()?;
        let subcommand = positionals.get(1).copied();
        let operation = match (root, subcommand) {
            ("init", _) => "init",
            ("project", Some("create")) => "project_create",
            ("project", Some("list")) => "project_list",
            ("project", _) => "project",
            ("issue", Some("create")) => "issue_create",
            ("issue", Some("batch")) => "issue_batch",
            ("issue", Some("show")) => "issue_show",
            ("issue", Some("list")) => "issue_list",
            ("issue", Some("edit")) => "issue_edit",
            ("issue", Some("comment")) => "issue_comment",
            ("issue", Some("history")) => "issue_history",
            ("issue", Some("dependency")) => "issue_dependency",
            ("issue", Some("parent")) => "issue_parent",
            ("issue", Some("claim")) => "issue_claim",
            ("issue", Some("heartbeat")) => "issue_heartbeat",
            ("issue", Some("takeover")) => "issue_takeover",
            ("decision", Some("request")) => "decision_request",
            ("decision", Some("resolve")) => "decision_resolve",
            ("event", Some("list")) => "event_list",
            ("capabilities", _) => "capabilities",
            ("issue", Some("start")) => "issue_start",
            ("issue", Some("block")) => "issue_block",
            ("issue", Some("resume")) => "issue_resume",
            ("issue", Some("complete")) => "issue_complete",
            ("issue", Some("cancel")) => "issue_cancel",
            ("issue", Some("reopen")) => "issue_reopen",
            ("issue", _) => "issue",
            ("status", _) => "status",
            ("audit", Some("list")) => "audit_list",
            ("audit", _) => "audit",
            ("context", _) => "context",
            ("web", _) => "web",
            _ => return None,
        };
        Some(Self {
            operation,
            database,
            project,
        })
    }
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    Init(InitCommand),
    Project(ProjectCommand),
    Issue(IssueCommand),
    Decision(DecisionCommand),
    Event(EventCommand),
    Capabilities(CapabilitiesCommand),
    Status(StatusCommand),
    Audit(AuditCommand),
    Context(ContextCommand),
    Web(WebCommand),
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
    Batch(IssueBatchCommand),
    Show(IssueShowCommand),
    List(IssueListCommand),
    Edit(IssueEditCommand),
    Comment(IssueCommentCommand),
    History(IssueHistoryCommand),
    Dependency(IssueDependencyCommand),
    Parent(IssueParentCommand),
    Claim(IssueClaimCommand),
    Heartbeat(IssueHeartbeatCommand),
    Takeover(IssueTakeoverCommand),
    Start(IssueStartCommand),
    Block(IssueBlockCommand),
    Resume(IssueResumeCommand),
    Complete(IssueCompleteCommand),
    Cancel(IssueCancelCommand),
    Reopen(IssueReopenCommand),
}

#[derive(clap::Args, Debug)]
pub struct IssueBatchCommand {
    #[arg(long, value_name = "PATH")]
    pub input: std::path::PathBuf,
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
pub struct IssueDependencyCommand {
    #[command(subcommand)]
    pub command: IssueDependencySubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum IssueDependencySubcommand {
    Add(IssueRelationCommand),
    Remove(IssueRelationCommand),
    List(IssueReferenceCommand),
}

#[derive(clap::Args, Debug)]
pub struct IssueRelationCommand {
    pub blocker: String,
    pub blocked: String,
}

#[derive(clap::Args, Debug)]
pub struct IssueReferenceCommand {
    pub reference: String,
}

#[derive(clap::Args, Debug)]
pub struct IssueParentCommand {
    #[command(subcommand)]
    pub command: IssueParentSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum IssueParentSubcommand {
    Set(IssueParentSetCommand),
    List(IssueReferenceCommand),
}

#[derive(clap::Args, Debug)]
pub struct IssueParentSetCommand {
    pub child: String,
    pub parent: String,
}

#[derive(clap::Args, Debug)]
pub struct IssueClaimCommand {
    pub number: Option<i64>,
}

#[derive(clap::Args, Debug)]
pub struct IssueHeartbeatCommand {
    pub number: i64,
}

#[derive(clap::Args, Debug)]
pub struct IssueTakeoverCommand {
    pub number: i64,

    #[arg(long)]
    pub reason: String,
}

#[derive(clap::Args, Debug)]
pub struct DecisionCommand {
    #[command(subcommand)]
    pub command: DecisionSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum DecisionSubcommand {
    Request(DecisionRequestCommand),
    Resolve(DecisionResolveCommand),
}

#[derive(clap::Args, Debug)]
pub struct DecisionRequestCommand {
    pub number: i64,

    #[arg(long)]
    pub blocker: String,

    #[arg(long)]
    pub question: String,

    #[arg(long = "option", required = true)]
    pub options: Vec<String>,

    #[arg(long)]
    pub recommendation: String,

    #[arg(long = "resume-condition")]
    pub resume_condition: String,

    #[arg(long)]
    pub background: String,
}

#[derive(clap::Args, Debug)]
pub struct DecisionResolveCommand {
    pub request_id: String,

    #[arg(long)]
    pub answer: String,

    #[arg(long)]
    pub next_state: crate::domain::IssueState,

    #[arg(long)]
    pub summary: Option<String>,

    #[arg(long)]
    pub verification: Option<String>,

    #[arg(long)]
    pub reason: Option<String>,

    #[arg(long, value_enum)]
    pub wait_kind: Option<crate::domain::WaitKind>,
}

#[derive(clap::Args, Debug)]
pub struct EventCommand {
    #[command(subcommand)]
    pub command: EventSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum EventSubcommand {
    List(EventListCommand),
}

#[derive(clap::Args, Debug)]
pub struct EventListCommand {
    #[arg(long)]
    pub after: i64,

    #[arg(long)]
    pub limit: Option<i64>,

    #[arg(long)]
    pub include_issue: bool,
}

#[derive(clap::Args, Debug)]
pub struct CapabilitiesCommand {}

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

#[derive(clap::Args, Debug)]
pub struct AuditCommand {
    #[command(subcommand)]
    pub command: AuditSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum AuditSubcommand {
    List(AuditListCommand),
}

#[derive(clap::Args, Debug)]
pub struct AuditListCommand {
    #[arg(long)]
    pub project_id: Option<uuid::Uuid>,

    #[arg(long)]
    pub operation: Option<String>,

    #[arg(long)]
    pub outcome: Option<String>,

    #[arg(long)]
    pub kind: Option<String>,

    #[arg(long)]
    pub agent: Option<String>,

    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long, value_parser = parse_timestamp)]
    pub after: Option<chrono::DateTime<chrono::Utc>>,

    #[arg(long, value_parser = parse_timestamp)]
    pub before: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(clap::Args, Debug)]
pub struct ContextCommand {}

#[derive(clap::Args, Debug)]
pub struct WebCommand {
    /// Port for the loopback-only web server. Use 0 to select an available port.
    #[arg(long, default_value_t = 4242)]
    pub port: u16,
}

fn parse_timestamp(value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    value
        .parse::<chrono::DateTime<chrono::Utc>>()
        .map_err(|_| "must be an RFC 3339 timestamp".to_owned())
}
