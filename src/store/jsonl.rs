#[derive(Clone, Debug)]
pub(crate) struct SourceAuditEvent {
    pub(crate) sequence: i64,
    pub(crate) event_id: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: String,
    pub(crate) operation: String,
    pub(crate) success: bool,
    pub(crate) exit_code: u8,
    pub(crate) initiator_kind: Option<String>,
    pub(crate) initiator_name: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) target_type: Option<String>,
    pub(crate) target_id: Option<String>,
    pub(crate) revision: Option<i64>,
    pub(crate) changed_fields_json: String,
    pub(crate) metadata_json: String,
}

impl SourceAuditEvent {
    pub(crate) fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, crate::error::AppError> {
        let success = row.get::<_, i64>(5).map_err(crate::error::AppError::from)?;
        let exit_code = row
            .get::<_, i64>(6)
            .map_err(crate::error::AppError::from)
            .and_then(|value| {
                u8::try_from(value).map_err(|_| {
                    crate::error::AppError::Internal(
                        "audit event exit code is outside the JSONL range".to_owned(),
                    )
                })
            })?;
        Ok(Self {
            sequence: row.get(0).map_err(crate::error::AppError::from)?,
            event_id: row.get(1).map_err(crate::error::AppError::from)?,
            started_at: row.get(2).map_err(crate::error::AppError::from)?,
            finished_at: row.get(3).map_err(crate::error::AppError::from)?,
            operation: row.get(4).map_err(crate::error::AppError::from)?,
            success: success != 0,
            exit_code,
            initiator_kind: row.get(7).map_err(crate::error::AppError::from)?,
            initiator_name: row.get(8).map_err(crate::error::AppError::from)?,
            session_id: row.get(9).map_err(crate::error::AppError::from)?,
            project_id: row.get(10).map_err(crate::error::AppError::from)?,
            target_type: row.get(11).map_err(crate::error::AppError::from)?,
            target_id: row.get(12).map_err(crate::error::AppError::from)?,
            revision: row.get(13).map_err(crate::error::AppError::from)?,
            changed_fields_json: row.get(14).map_err(crate::error::AppError::from)?,
            metadata_json: row.get(15).map_err(crate::error::AppError::from)?,
        })
    }

    pub(crate) fn to_jsonl(
        &self,
        previous_hash: Option<&str>,
    ) -> Result<JsonlEvent, crate::error::AppError> {
        let context = self.execution_context()?;
        let changed_fields = serde_json::from_str::<Vec<String>>(&self.changed_fields_json)
            .map_err(|_| {
                crate::error::AppError::Internal(
                    "audit event changed fields are not valid JSON".to_owned(),
                )
            })?;
        let error_code = if self.success {
            None
        } else {
            serde_json::from_str::<serde_json::Value>(&self.metadata_json)
                .ok()
                .and_then(|metadata| {
                    metadata
                        .get("error_code")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned)
                })
        };
        let body = JsonlEventBody {
            schema_version: 1,
            sequence: self.sequence,
            event_id: self.event_id.clone(),
            started_at: self.started_at.clone(),
            finished_at: self.finished_at.clone(),
            operation: self.operation.clone(),
            context,
            project_id: self.project_id.clone(),
            target: self
                .target_type
                .clone()
                .zip(self.target_id.clone())
                .map(|(kind, id)| JsonlTarget { kind, id }),
            revision: self.revision,
            changed_fields,
            result: JsonlResult {
                outcome: if self.success { "success" } else { "failure" },
                exit_code: self.exit_code,
                error_code,
            },
            previous_hash: previous_hash.map(str::to_owned),
        };
        let mut value = serde_json::to_value(body)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        value = canonicalize(value);
        let encoded = serde_json::to_vec(&value)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        let hash = sha256_hex(&encoded);
        value
            .as_object_mut()
            .expect("JSONL event body is always an object")
            .insert("hash".to_owned(), serde_json::Value::String(hash.clone()));
        let line = serde_json::to_string(&value)
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
        Ok(JsonlEvent {
            sequence: self.sequence,
            event_id: self.event_id.clone(),
            previous_hash: previous_hash.map(str::to_owned),
            hash,
            line,
        })
    }

    fn execution_context(&self) -> Result<crate::domain::ExecutionContext, crate::error::AppError> {
        let kind = match self.initiator_kind.as_deref() {
            Some("agent") => crate::domain::InitiatorKind::Agent,
            Some("human") => crate::domain::InitiatorKind::Human,
            Some("system") | None => crate::domain::InitiatorKind::System,
            Some(_) => {
                return Err(crate::error::AppError::Internal(
                    "audit event initiator kind is invalid".to_owned(),
                ));
            }
        };
        Ok(crate::domain::ExecutionContext {
            kind,
            agent: (kind == crate::domain::InitiatorKind::Agent)
                .then(|| self.initiator_name.clone())
                .flatten(),
            session_id: self.session_id.clone(),
            operator: (kind == crate::domain::InitiatorKind::Human)
                .then(|| self.initiator_name.clone())
                .flatten(),
        })
    }
}

#[derive(Clone, Debug, serde::Serialize)]
struct JsonlEventBody {
    schema_version: u32,
    sequence: i64,
    event_id: String,
    started_at: String,
    finished_at: String,
    operation: String,
    context: crate::domain::ExecutionContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<JsonlTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<i64>,
    changed_fields: Vec<String>,
    result: JsonlResult,
    previous_hash: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct JsonlTarget {
    kind: String,
    id: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct JsonlResult {
    outcome: &'static str,
    exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct JsonlEvent {
    pub(crate) sequence: i64,
    pub(crate) event_id: String,
    pub(crate) previous_hash: Option<String>,
    pub(crate) hash: String,
    pub(crate) line: String,
}

impl JsonlEvent {
    pub(crate) fn matches(&self, existing: &ExistingJsonlEvent) -> bool {
        self.sequence == existing.sequence
            && self.event_id == existing.event_id
            && self.previous_hash == existing.previous_hash
            && self.hash == existing.hash
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ExistingJsonlEvent {
    pub(crate) sequence: i64,
    pub(crate) event_id: String,
    pub(crate) previous_hash: Option<String>,
    pub(crate) hash: String,
}

pub(crate) fn read_existing(
    file: &mut std::fs::File,
) -> Result<std::collections::BTreeMap<i64, ExistingJsonlEvent>, crate::error::AppError> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    file.seek(SeekFrom::Start(0))
        .map_err(|_| crate::error::AppError::Internal("audit JSONL read failed".to_owned()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| crate::error::AppError::Internal("audit JSONL read failed".to_owned()))?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        let valid_length = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        file.set_len(valid_length as u64).map_err(|_| {
            crate::error::AppError::Internal("audit JSONL repair failed".to_owned())
        })?;
        bytes.truncate(valid_length);
    }

    let mut events = std::collections::BTreeMap::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let event = serde_json::from_slice::<ExistingJsonlEvent>(line).map_err(|_| {
            crate::error::AppError::Internal("audit JSONL contains an invalid line".to_owned())
        })?;
        if events.insert(event.sequence, event).is_some() {
            return Err(crate::error::AppError::Internal(
                "audit JSONL contains duplicate sequences".to_owned(),
            ));
        }
    }
    Ok(events)
}

pub(crate) fn append(
    file: &mut std::fs::File,
    event: &JsonlEvent,
) -> Result<(), crate::error::AppError> {
    use std::io::{Seek as _, SeekFrom, Write as _};

    file.seek(SeekFrom::End(0))
        .map_err(|_| crate::error::AppError::Internal("audit JSONL append failed".to_owned()))?;
    file.write_all(event.line.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|_| crate::error::AppError::Internal("audit JSONL append failed".to_owned()))?;
    file.sync_data()
        .map_err(|_| crate::error::AppError::Internal("audit JSONL sync failed".to_owned()))
}

pub(crate) fn path_for_database(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension("audit.jsonl")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;

    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonicalize(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize(value)))
                    .collect(),
            )
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize).collect())
        }
        value => value,
    }
}
