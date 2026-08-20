#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InitiatorKind {
    Agent,
    Human,
    #[allow(dead_code)]
    System,
}

impl InitiatorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ExecutionContext {
    pub kind: InitiatorKind,
    pub agent: Option<String>,
    pub session_id: Option<String>,
    pub operator: Option<String>,
}

impl ExecutionContext {
    pub fn resolve() -> Result<Self, crate::error::AppError> {
        if let Some(agent) = Self::environment_value("BETTR_AGENT")? {
            return Ok(Self {
                kind: InitiatorKind::Agent,
                agent: Some(agent),
                session_id: Self::environment_value("BETTR_SESSION_ID")?,
                operator: None,
            });
        }

        let operator = match Self::environment_value("BETTR_OPERATOR")? {
            Some(operator) => operator,
            None => Self::validate_value("OS username", whoami::username())?,
        };
        Ok(Self {
            kind: InitiatorKind::Human,
            agent: None,
            session_id: None,
            operator: Some(operator),
        })
    }

    pub fn initiator_name(&self) -> Option<&str> {
        match self.kind {
            InitiatorKind::Agent => self.agent.as_deref(),
            InitiatorKind::Human => self.operator.as_deref(),
            InitiatorKind::System => None,
        }
    }

    fn environment_value(name: &str) -> Result<Option<String>, crate::error::AppError> {
        match std::env::var_os(name) {
            Some(value) => value
                .into_string()
                .map_err(|_| {
                    crate::error::AppError::InvalidInput(format!("{name} must be valid Unicode"))
                })
                .and_then(|value| Self::validate_value(name, value).map(Some)),
            None => Ok(None),
        }
    }

    fn validate_value(name: &str, value: String) -> Result<String, crate::error::AppError> {
        if value.trim().is_empty() {
            return Err(crate::error::AppError::InvalidInput(format!(
                "{name} must not be empty"
            )));
        }
        if value.chars().count() > 200 {
            return Err(crate::error::AppError::InvalidInput(format!(
                "{name} must contain at most 200 Unicode scalar values"
            )));
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Project {
    pub id: uuid::Uuid,
    pub name: String,
    pub archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum IssueState {
    Todo,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

impl IssueState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Result<Self, crate::error::AppError> {
        match value {
            "todo" => Ok(Self::Todo),
            "in_progress" => Ok(Self::InProgress),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(crate::error::AppError::Internal(format!(
                "invalid issue state in database: {value}"
            ))),
        }
    }

    pub fn apply(&self, transition: &Transition) -> Result<Self, DomainError> {
        match (*self, transition) {
            (Self::Todo, Transition::Start) => Ok(Self::InProgress),
            (Self::InProgress, Transition::Block(_)) => Ok(Self::Blocked),
            (Self::Blocked, Transition::Resume) => Ok(Self::InProgress),
            (Self::InProgress, Transition::Complete(_)) => Ok(Self::Done),
            (Self::InProgress | Self::Blocked, Transition::Cancel(_)) => Ok(Self::Cancelled),
            (Self::Done | Self::Cancelled, Transition::Reopen(_)) => Ok(Self::Todo),
            _ => Err(DomainError::InvalidTransition {
                from: *self,
                transition: transition.name(),
            }),
        }
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum WaitKind {
    Human,
    Dependency,
    External,
}

impl WaitKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Dependency => "dependency",
            Self::External => "external",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockTransition {
    reason: String,
    wait_kind: WaitKind,
}

#[derive(Clone, Debug)]
pub struct CompleteTransition {
    summary: String,
    verification: String,
}

#[derive(Clone, Debug)]
pub struct ReasonTransition {
    reason: String,
}

#[derive(Clone, Debug)]
pub enum Transition {
    Start,
    Block(BlockTransition),
    Resume,
    Complete(CompleteTransition),
    Cancel(ReasonTransition),
    Reopen(ReasonTransition),
}

impl Transition {
    pub fn block(reason: String, wait_kind: WaitKind) -> Result<Self, DomainError> {
        validate_transition_metadata("block reason", &reason)?;
        Ok(Self::Block(BlockTransition { reason, wait_kind }))
    }

    pub fn complete(summary: String, verification: String) -> Result<Self, DomainError> {
        validate_transition_metadata("completion summary", &summary)?;
        validate_transition_metadata("completion verification", &verification)?;
        Ok(Self::Complete(CompleteTransition {
            summary,
            verification,
        }))
    }

    pub fn cancel(reason: String) -> Result<Self, DomainError> {
        validate_transition_metadata("cancellation reason", &reason)?;
        Ok(Self::Cancel(ReasonTransition { reason }))
    }

    pub fn reopen(reason: String) -> Result<Self, DomainError> {
        validate_transition_metadata("reopen reason", &reason)?;
        Ok(Self::Reopen(ReasonTransition { reason }))
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Block(_) => "block",
            Self::Resume => "resume",
            Self::Complete(_) => "complete",
            Self::Cancel(_) => "cancel",
            Self::Reopen(_) => "reopen",
        }
    }

    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::Start => "issue_started",
            Self::Block(_) => "issue_blocked",
            Self::Resume => "issue_resumed",
            Self::Complete(_) => "issue_completed",
            Self::Cancel(_) => "issue_cancelled",
            Self::Reopen(_) => "issue_reopened",
        }
    }

    pub const fn operation(&self) -> &'static str {
        match self {
            Self::Start => "issue_start",
            Self::Block(_) => "issue_block",
            Self::Resume => "issue_resume",
            Self::Complete(_) => "issue_complete",
            Self::Cancel(_) => "issue_cancel",
            Self::Reopen(_) => "issue_reopen",
        }
    }

    pub fn idempotency_payload(&self) -> serde_json::Value {
        match self {
            Self::Start => serde_json::json!({ "kind": "start" }),
            Self::Block(block) => serde_json::json!({
                "kind": "block",
                "reason": block.reason,
                "wait_kind": block.wait_kind,
            }),
            Self::Resume => serde_json::json!({ "kind": "resume" }),
            Self::Complete(complete) => serde_json::json!({
                "kind": "complete",
                "summary": complete.summary,
                "verification": complete.verification,
            }),
            Self::Cancel(cancel) => {
                serde_json::json!({ "kind": "cancel", "reason": cancel.reason })
            }
            Self::Reopen(reopen) => {
                serde_json::json!({ "kind": "reopen", "reason": reopen.reason })
            }
        }
    }

    pub const fn changed_fields(&self) -> &'static [&'static str] {
        match self {
            Self::Start | Self::Resume => &["state"],
            Self::Block(_) => &["state", "reason", "wait_kind"],
            Self::Complete(_) => &["state", "summary", "verification"],
            Self::Cancel(_) | Self::Reopen(_) => &["state", "reason"],
        }
    }

    pub fn event_metadata(
        &self,
        from_state: IssueState,
        to_state: IssueState,
        revision: i64,
    ) -> serde_json::Value {
        let mut metadata = serde_json::json!({
            "from_state": from_state,
            "to_state": to_state,
            "revision": revision,
        });
        let object = metadata.as_object_mut().expect("object literal");
        match self {
            Self::Start | Self::Resume => {}
            Self::Block(block) => {
                object.insert("reason".to_owned(), block.reason.clone().into());
                object.insert("wait_kind".to_owned(), block.wait_kind.as_str().into());
            }
            Self::Complete(complete) => {
                object.insert("summary".to_owned(), complete.summary.clone().into());
                object.insert(
                    "verification".to_owned(),
                    complete.verification.clone().into(),
                );
            }
            Self::Cancel(cancel) | Self::Reopen(cancel) => {
                object.insert("reason".to_owned(), cancel.reason.clone().into());
            }
        }
        metadata
    }
}

#[derive(Clone, Debug)]
pub struct DecisionResolution {
    target_state: IssueState,
    transition: Option<Transition>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct DecisionResolutionInput {
    target_state: IssueState,
    summary: Option<String>,
    verification: Option<String>,
    reason: Option<String>,
    wait_kind: Option<WaitKind>,
}

impl DecisionResolutionInput {
    pub fn new(
        target_state: IssueState,
        summary: Option<String>,
        verification: Option<String>,
        reason: Option<String>,
        wait_kind: Option<WaitKind>,
    ) -> Self {
        Self {
            target_state,
            summary,
            verification,
            reason,
            wait_kind,
        }
    }

    pub const fn target_state(&self) -> IssueState {
        self.target_state
    }

    pub fn into_resolution(self) -> Result<DecisionResolution, DomainError> {
        DecisionResolution::new(
            self.target_state,
            self.summary,
            self.verification,
            self.reason,
            self.wait_kind,
        )
    }
}

impl DecisionResolution {
    pub fn new(
        target_state: IssueState,
        summary: Option<String>,
        verification: Option<String>,
        reason: Option<String>,
        wait_kind: Option<WaitKind>,
    ) -> Result<Self, DomainError> {
        let transition = match target_state {
            IssueState::Todo => {
                if summary.is_some()
                    || verification.is_some()
                    || reason.is_some()
                    || wait_kind.is_some()
                {
                    return Err(DomainError::InvalidMetadata(
                        "decision resolution to todo does not accept transition metadata"
                            .to_owned(),
                    ));
                }
                None
            }
            IssueState::Blocked => {
                if summary.is_some() || verification.is_some() {
                    return Err(DomainError::InvalidMetadata(
                        "decision resolution to blocked does not accept summary or verification"
                            .to_owned(),
                    ));
                }
                let reason = reason.ok_or_else(|| {
                    DomainError::InvalidMetadata(
                        "decision resolution to blocked requires --reason".to_owned(),
                    )
                })?;
                let wait_kind = wait_kind.ok_or_else(|| {
                    DomainError::InvalidMetadata(
                        "decision resolution to blocked requires --wait-kind".to_owned(),
                    )
                })?;
                Some(Transition::block(reason, wait_kind)?)
            }
            IssueState::Done => {
                if reason.is_some() || wait_kind.is_some() {
                    return Err(DomainError::InvalidMetadata(
                        "decision resolution to done does not accept reason or wait kind"
                            .to_owned(),
                    ));
                }
                let summary = summary.ok_or_else(|| {
                    DomainError::InvalidMetadata(
                        "decision resolution to done requires --summary".to_owned(),
                    )
                })?;
                let verification = verification.ok_or_else(|| {
                    DomainError::InvalidMetadata(
                        "decision resolution to done requires --verification".to_owned(),
                    )
                })?;
                Some(Transition::complete(summary, verification)?)
            }
            IssueState::Cancelled => {
                if summary.is_some() || verification.is_some() || wait_kind.is_some() {
                    return Err(DomainError::InvalidMetadata(
                        "decision resolution to cancelled does not accept summary, verification, or wait kind"
                            .to_owned(),
                    ));
                }
                let reason = reason.ok_or_else(|| {
                    DomainError::InvalidMetadata(
                        "decision resolution to cancelled requires --reason".to_owned(),
                    )
                })?;
                Some(Transition::cancel(reason)?)
            }
            IssueState::InProgress => {
                return Err(DomainError::InvalidTransition {
                    from: IssueState::Blocked,
                    transition: "decision_resolve",
                });
            }
        };
        Ok(Self {
            target_state,
            transition,
        })
    }

    pub const fn target_state(&self) -> IssueState {
        self.target_state
    }

    pub const fn event_type(&self) -> &'static str {
        match self.target_state {
            IssueState::Todo => "decision_resolved",
            IssueState::Blocked => "issue_blocked",
            IssueState::Done => "issue_completed",
            IssueState::Cancelled => "issue_cancelled",
            IssueState::InProgress => "decision_resolved",
        }
    }

    pub const fn changed_fields(&self) -> &'static [&'static str] {
        match self.target_state {
            IssueState::Todo => &["decision", "state"],
            IssueState::Blocked => &["decision", "state", "reason", "wait_kind"],
            IssueState::Done => &["decision", "state", "summary", "verification"],
            IssueState::Cancelled => &["decision", "state", "reason"],
            IssueState::InProgress => &["decision", "state"],
        }
    }

    pub fn event_metadata(&self, from_state: IssueState, revision: i64) -> serde_json::Value {
        match &self.transition {
            Some(transition) => transition.event_metadata(from_state, self.target_state, revision),
            None => serde_json::json!({
                "from_state": from_state,
                "to_state": self.target_state,
                "revision": revision,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    InvalidMetadata(String),
    InvalidTransition {
        from: IssueState,
        transition: &'static str,
    },
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMetadata(message) => formatter.write_str(message),
            Self::InvalidTransition { from, transition } => write!(
                formatter,
                "cannot {transition} an issue in the {} state",
                from.as_str()
            ),
        }
    }
}

impl std::error::Error for DomainError {}

fn validate_transition_metadata(name: &str, value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::InvalidMetadata(format!(
            "{name} must not be empty"
        )));
    }
    Ok(())
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum AssigneeKind {
    Human,
    Agent,
}

impl AssigneeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Result<Self, crate::error::AppError> {
        match value {
            "human" => Ok(Self::Human),
            "agent" => Ok(Self::Agent),
            _ => Err(crate::error::AppError::Internal(format!(
                "invalid assignee kind in database: {value}"
            ))),
        }
    }
}

impl Priority {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn parse(value: &str) -> Result<Self, crate::error::AppError> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" => Ok(Self::Critical),
            _ => Err(crate::error::AppError::Internal(format!(
                "invalid issue priority in database: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Issue {
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: IssueState,
    pub priority: Option<Priority>,
    pub assignee_kind: Option<AssigneeKind>,
    pub assignee_name: Option<String>,
    pub revision: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct IssueFilter {
    pub projects: Vec<String>,
    pub states: Vec<IssueState>,
    pub priorities: Vec<Priority>,
    pub assignee: Option<String>,
    pub updated_after: Option<chrono::DateTime<chrono::Utc>>,
    pub query: Option<String>,
    pub include_done: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct IssueListItem {
    pub project: String,
    #[serde(flatten)]
    pub issue: Issue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueReference {
    pub project: String,
    pub number: i64,
}

impl IssueReference {
    pub fn parse(
        value: &str,
        default_project: Option<&str>,
    ) -> Result<Self, crate::error::AppError> {
        let (project, number) = match value.rsplit_once('#') {
            Some((project, number)) => (project.to_owned(), number),
            None => (
                default_project
                    .ok_or_else(|| {
                        crate::error::AppError::InvalidInput(
                            "issue reference must use PROJECT#NUMBER or provide --project"
                                .to_owned(),
                        )
                    })?
                    .to_owned(),
                value,
            ),
        };
        validate_project_name(&project)?;
        let number = number.parse::<i64>().map_err(|_| {
            crate::error::AppError::InvalidInput(
                "issue reference number must be a positive integer".to_owned(),
            )
        })?;
        if number < 1 {
            return Err(crate::error::AppError::InvalidInput(
                "issue reference number must be positive".to_owned(),
            ));
        }
        Ok(Self { project, number })
    }

    pub fn label(&self) -> String {
        format!("{}#{}", self.project, self.number)
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct IssueDependency {
    pub blocker: String,
    pub blocked: String,
    pub relation: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct IssueParent {
    pub child: String,
    pub parent: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct IssueLease {
    pub agent: String,
    pub session_id: String,
    pub claimed_at: chrono::DateTime<chrono::Utc>,
    pub heartbeat_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub lease_revision: i64,
    pub stale: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ClaimedIssue {
    pub issue: Issue,
    pub lease: IssueLease,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DecisionRequestInput {
    pub blocker: String,
    pub question: String,
    pub options: Vec<String>,
    pub recommendation: String,
    pub resume_condition: String,
    pub background: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DecisionRequest {
    pub id: uuid::Uuid,
    pub issue: String,
    pub question: String,
    pub background: String,
    pub blocker: String,
    pub options: Vec<String>,
    pub recommendation: String,
    pub resume_condition: String,
    pub requester_kind: Option<InitiatorKind>,
    pub requester_name: Option<String>,
    pub requester_session_id: Option<String>,
    pub status: String,
    pub answer: Option<String>,
    pub resolver_kind: Option<InitiatorKind>,
    pub resolver_name: Option<String>,
    pub resolver_session_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct EventRecord {
    pub sequence: i64,
    pub event_type: String,
    pub project_id: Option<uuid::Uuid>,
    pub issue_id: Option<uuid::Uuid>,
    pub changed_fields: Vec<String>,
    pub revision: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub issue: Option<Issue>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct EventPage {
    pub next_cursor: i64,
    pub has_more: bool,
    pub events: Vec<EventRecord>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CapabilitySet {
    pub json_contract_version: u32,
    pub cli_version: &'static str,
    pub capabilities: std::collections::BTreeMap<&'static str, bool>,
}

pub fn validate_decision_text(name: &str, value: &str) -> Result<(), crate::error::AppError> {
    if value.trim().is_empty() {
        return Err(crate::error::AppError::InvalidInput(format!(
            "{name} must not be empty"
        )));
    }
    Ok(())
}

pub fn validate_decision_options(options: &[String]) -> Result<(), crate::error::AppError> {
    if options.len() < 2 {
        return Err(crate::error::AppError::InvalidInput(
            "decision options must contain at least two choices".to_owned(),
        ));
    }
    for (index, option) in options.iter().enumerate() {
        validate_decision_text(&format!("decision option {}", index + 1), option)?;
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    pub attention: Vec<IssueListItem>,
    pub stale: Vec<IssueListItem>,
    pub blocked: Vec<IssueListItem>,
    pub recently_completed: Vec<IssueListItem>,
    pub active: Vec<IssueListItem>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct NewIssue {
    pub title: String,
    pub body: Option<String>,
    pub priority: Option<Priority>,
}

impl NewIssue {
    pub fn validate(&self) -> Result<(), crate::error::AppError> {
        validate_issue_title(&self.title)?;
        validate_issue_body(self.body.as_deref())
    }

    pub fn changed_fields(&self) -> Vec<&'static str> {
        let mut fields = vec!["title"];
        if self.body.is_some() {
            fields.push("body");
        }
        fields.push("state");
        if self.priority.is_some() {
            fields.push("priority");
        }
        fields
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<Option<String>>,
    pub priority: Option<Option<Priority>>,
    pub assignee_kind: Option<Option<AssigneeKind>>,
    pub assignee_name: Option<Option<String>>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum BatchOperation {
    IssueCreate {
        project: Option<String>,
        title: String,
        body: Option<String>,
        priority: Option<Priority>,
    },
    IssueEdit {
        project: Option<String>,
        number: i64,
        revision: i64,
        patch: IssuePatch,
    },
    IssueComment {
        project: Option<String>,
        number: i64,
        body: String,
    },
    IssueStart {
        project: Option<String>,
        number: i64,
        revision: i64,
    },
    IssueBlock {
        project: Option<String>,
        number: i64,
        revision: i64,
        reason: String,
        wait_kind: WaitKind,
    },
    IssueResume {
        project: Option<String>,
        number: i64,
        revision: i64,
    },
    IssueComplete {
        project: Option<String>,
        number: i64,
        revision: i64,
        summary: String,
        verification: String,
    },
    IssueCancel {
        project: Option<String>,
        number: i64,
        revision: i64,
        reason: String,
    },
    IssueReopen {
        project: Option<String>,
        number: i64,
        revision: i64,
        reason: String,
    },
}

impl BatchOperation {
    pub const fn operation_name(&self) -> &'static str {
        match self {
            Self::IssueCreate { .. } => "issue_create",
            Self::IssueEdit { .. } => "issue_edit",
            Self::IssueComment { .. } => "issue_comment",
            Self::IssueStart { .. } => "issue_start",
            Self::IssueBlock { .. } => "issue_block",
            Self::IssueResume { .. } => "issue_resume",
            Self::IssueComplete { .. } => "issue_complete",
            Self::IssueCancel { .. } => "issue_cancel",
            Self::IssueReopen { .. } => "issue_reopen",
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BatchResult {
    pub operation: String,
    pub result: serde_json::Value,
}

impl IssuePatch {
    pub fn validate(&self) -> Result<(), crate::error::AppError> {
        if self.title.is_none()
            && self.body.is_none()
            && self.priority.is_none()
            && self.assignee_kind.is_none()
            && self.assignee_name.is_none()
        {
            return Err(crate::error::AppError::InvalidInput(
                "issue edit must change at least one field".to_owned(),
            ));
        }
        if let Some(title) = &self.title {
            validate_issue_title(title)?;
        }
        if let Some(body) = &self.body {
            validate_issue_body(body.as_deref())?;
        }
        if self.assignee_kind.is_some() != self.assignee_name.is_some() {
            return Err(crate::error::AppError::InvalidInput(
                "assignee kind and name must be provided together".to_owned(),
            ));
        }
        if let (Some(kind), Some(name)) = (&self.assignee_kind, &self.assignee_name) {
            if kind.is_some() != name.is_some() {
                return Err(crate::error::AppError::InvalidInput(
                    "assignee kind and name must be set or cleared together".to_owned(),
                ));
            }
            if let Some(name) = name {
                if name.trim().is_empty() {
                    return Err(crate::error::AppError::InvalidInput(
                        "assignee name must not be empty".to_owned(),
                    ));
                }
                if name.chars().count() > 200 {
                    return Err(crate::error::AppError::InvalidInput(
                        "assignee name must contain at most 200 Unicode scalar values".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn apply_to(&self, issue: &mut Issue) {
        if let Some(title) = &self.title {
            issue.title.clone_from(title);
        }
        if let Some(body) = &self.body {
            issue.body.clone_from(body);
        }
        if let Some(priority) = self.priority {
            issue.priority = priority;
        }
        if let Some(assignee_kind) = self.assignee_kind {
            issue.assignee_kind = assignee_kind;
        }
        if let Some(assignee_name) = &self.assignee_name {
            issue.assignee_name.clone_from(assignee_name);
        }
    }

    pub fn changed_fields(&self, before: &Issue, after: &Issue) -> Vec<&'static str> {
        let mut fields = Vec::new();
        if self.title.is_some() && before.title != after.title {
            fields.push("title");
        }
        if self.body.is_some() && before.body != after.body {
            fields.push("body");
        }
        if self.priority.is_some() && before.priority != after.priority {
            fields.push("priority");
        }
        if self.assignee_kind.is_some() && before.assignee_kind != after.assignee_kind {
            fields.push("assignee_kind");
        }
        if self.assignee_name.is_some() && before.assignee_name != after.assignee_name {
            fields.push("assignee_name");
        }
        fields
    }

    pub fn event_metadata(&self, revision: i64) -> serde_json::Value {
        let mut changes = serde_json::Map::new();
        if let Some(title) = &self.title {
            changes.insert("title".to_owned(), title.clone().into());
        }
        if let Some(body) = &self.body {
            changes.insert(
                "body".to_owned(),
                body.as_ref()
                    .map_or(serde_json::Value::Null, |body| body.clone().into()),
            );
        }
        if let Some(priority) = self.priority {
            changes.insert(
                "priority".to_owned(),
                priority.map_or(serde_json::Value::Null, |priority| priority.as_str().into()),
            );
        }
        if let Some(assignee_kind) = self.assignee_kind {
            changes.insert(
                "assignee_kind".to_owned(),
                assignee_kind.map_or(serde_json::Value::Null, |kind| kind.as_str().into()),
            );
        }
        if let Some(assignee_name) = &self.assignee_name {
            changes.insert(
                "assignee_name".to_owned(),
                assignee_name
                    .as_ref()
                    .map_or(serde_json::Value::Null, |name| name.clone().into()),
            );
        }
        serde_json::json!({ "changes": changes, "revision": revision })
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Comment {
    pub id: uuid::Uuid,
    pub issue_id: uuid::Uuid,
    pub body: String,
    pub context: ExecutionContext,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct DomainEvent {
    pub sequence: i64,
    pub event_type: String,
    pub revision: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub context: ExecutionContext,
    pub metadata: serde_json::Value,
}

pub fn validate_comment_body(body: &str) -> Result<(), crate::error::AppError> {
    if body.trim().is_empty() {
        return Err(crate::error::AppError::InvalidInput(
            "comment body must not be empty".to_owned(),
        ));
    }
    if body.len() > 1_048_576 {
        return Err(crate::error::AppError::InvalidInput(
            "comment body must contain at most 1 MiB".to_owned(),
        ));
    }
    Ok(())
}

fn validate_issue_title(title: &str) -> Result<(), crate::error::AppError> {
    if title.trim().is_empty() {
        return Err(crate::error::AppError::InvalidInput(
            "issue title must not be empty".to_owned(),
        ));
    }
    if title.chars().count() > 500 {
        return Err(crate::error::AppError::InvalidInput(
            "issue title must contain at most 500 Unicode scalar values".to_owned(),
        ));
    }
    Ok(())
}

fn validate_issue_body(body: Option<&str>) -> Result<(), crate::error::AppError> {
    if body.is_some_and(|body| body.len() > 1_048_576) {
        return Err(crate::error::AppError::InvalidInput(
            "issue body must contain at most 1 MiB".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_project_name(name: &str) -> Result<(), crate::error::AppError> {
    if name.trim().is_empty() {
        return Err(crate::error::AppError::InvalidInput(
            "project name must not be empty".to_owned(),
        ));
    }
    if name.chars().count() > 200 {
        return Err(crate::error::AppError::InvalidInput(
            "project name must contain at most 200 Unicode scalar values".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_idempotency_key(key: &str) -> Result<(), crate::error::AppError> {
    if key.trim().is_empty() {
        return Err(crate::error::AppError::InvalidInput(
            "idempotency key must not be empty".to_owned(),
        ));
    }
    if key.chars().count() > 200 {
        return Err(crate::error::AppError::InvalidInput(
            "idempotency key must contain at most 200 Unicode scalar values".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn issue_state_apply_uses_the_guarded_transition_table() {
        let allowed = [
            (
                super::IssueState::Todo,
                super::Transition::Start,
                super::IssueState::InProgress,
            ),
            (
                super::IssueState::Blocked,
                super::Transition::Resume,
                super::IssueState::InProgress,
            ),
        ];
        for (state, transition, expected) in allowed {
            assert_eq!(state.apply(&transition).unwrap(), expected);
        }

        assert!(matches!(
            super::IssueState::Done.apply(&super::Transition::Start),
            Err(super::DomainError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn transition_metadata_constructors_reject_blank_values() {
        assert!(matches!(
            super::Transition::block(" ".to_owned(), super::WaitKind::Human),
            Err(super::DomainError::InvalidMetadata(_))
        ));
        assert!(matches!(
            super::Transition::complete("done".to_owned(), " ".to_owned()),
            Err(super::DomainError::InvalidMetadata(_))
        ));
        assert!(matches!(
            super::Transition::cancel(" ".to_owned()),
            Err(super::DomainError::InvalidMetadata(_))
        ));
        assert!(matches!(
            super::Transition::reopen(" ".to_owned()),
            Err(super::DomainError::InvalidMetadata(_))
        ));
    }

    #[test]
    fn new_issue_rejects_a_body_larger_than_one_mebibyte() {
        let issue = super::NewIssue {
            title: "Build local core".to_owned(),
            body: Some("a".repeat(1_048_577)),
            priority: None,
        };

        assert!(matches!(
            issue.validate(),
            Err(crate::error::AppError::InvalidInput(message))
                if message == "issue body must contain at most 1 MiB"
        ));
    }

    #[test]
    fn idempotency_key_validation_rejects_blank_and_oversized_keys() {
        assert!(super::validate_idempotency_key(" ").is_err());
        assert!(super::validate_idempotency_key(&"a".repeat(201)).is_err());
        assert!(super::validate_idempotency_key("request-123").is_ok());
    }

    #[test]
    fn batch_operation_uses_a_stable_tagged_json_shape() {
        let operation: super::BatchOperation =
            serde_json::from_str(r#"{"operation":"issue_create","title":"new issue"}"#).unwrap();
        assert!(matches!(
            operation,
            super::BatchOperation::IssueCreate { .. }
        ));
        assert!(
            serde_json::from_str::<super::BatchOperation>(r#"{"operation":"unknown"}"#).is_err()
        );
    }
}
