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
