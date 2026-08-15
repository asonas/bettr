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

#[derive(Clone, Debug, serde::Serialize)]
pub struct Project {
    pub id: uuid::Uuid,
    pub name: String,
    pub archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Serialize)]
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
            Self::Urgent => "urgent",
        }
    }

    pub fn parse(value: &str) -> Result<Self, crate::error::AppError> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "urgent" => Ok(Self::Urgent),
            _ => Err(crate::error::AppError::Internal(format!(
                "invalid issue priority in database: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
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

#[derive(Clone, Debug, serde::Serialize)]
pub struct IssueListItem {
    pub project: String,
    #[serde(flatten)]
    pub issue: Issue,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    pub attention: Vec<IssueListItem>,
    pub stale: Vec<IssueListItem>,
    pub blocked: Vec<IssueListItem>,
    pub recently_completed: Vec<IssueListItem>,
    pub active: Vec<IssueListItem>,
}

#[derive(Clone, Debug)]
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
}

#[derive(Clone, Debug)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<Option<String>>,
    pub priority: Option<Option<Priority>>,
    pub assignee_kind: Option<Option<AssigneeKind>>,
    pub assignee_name: Option<Option<String>>,
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

#[derive(Clone, Debug, serde::Serialize)]
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
}
