#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
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

#[derive(Clone, Debug, serde::Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
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
    pub assignee_kind: Option<String>,
    pub assignee_name: Option<String>,
    pub revision: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct NewIssue {
    pub title: String,
    pub body: Option<String>,
    pub priority: Option<Priority>,
}

impl NewIssue {
    pub fn validate(&self) -> Result<(), crate::error::AppError> {
        if self.title.trim().is_empty() {
            return Err(crate::error::AppError::InvalidInput(
                "issue title must not be empty".to_owned(),
            ));
        }
        if self.title.chars().count() > 500 {
            return Err(crate::error::AppError::InvalidInput(
                "issue title must contain at most 500 Unicode scalar values".to_owned(),
            ));
        }
        if self
            .body
            .as_ref()
            .is_some_and(|body| body.len() > 1_048_576)
        {
            return Err(crate::error::AppError::InvalidInput(
                "issue body must contain at most 1 MiB".to_owned(),
            ));
        }
        Ok(())
    }
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
