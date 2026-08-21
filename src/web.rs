const INDEX_HTML: &str = include_str!("web/index.html");
const APP_CSS: &str = include_str!("web/app.css");
const STATE_JS: &str = include_str!("web/state.js");
const APP_JS: &str = include_str!("web/app.js");

const MAX_REQUEST_LINE: usize = 8 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_REQUEST_BODY: usize = 64 * 1024;

pub fn run(database_path: &std::path::Path, port: u16) -> Result<(), crate::error::AppError> {
    let _database = crate::store::Database::open(database_path)?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
        crate::error::AppError::Internal(format!("could not bind web server: {error}"))
    })?;
    let address = listener.local_addr().map_err(|error| {
        crate::error::AppError::Internal(format!("could not resolve web server address: {error}"))
    })?;
    println!("listening http://{address}");
    use std::io::Write as _;
    std::io::stdout().flush().map_err(|error| {
        crate::error::AppError::Internal(format!("could not flush web server address: {error}"))
    })?;

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(error) = handle_connection(&mut stream, database_path) {
                    eprintln!("web request failed: {error}");
                }
            }
            Err(error) => {
                return Err(crate::error::AppError::Internal(format!(
                    "web server accept failed: {error}"
                )));
            }
        }
    }

    Ok(())
}

fn handle_connection(
    stream: &mut std::net::TcpStream,
    database_path: &std::path::Path,
) -> Result<(), crate::error::AppError> {
    let request = read_request(stream)?;
    let response = route_request(database_path, &request);
    write_response(stream, &response)
}

struct Request {
    method: String,
    path: String,
    query: std::collections::BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct WebWait {
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<crate::domain::WaitKind>,
    label: String,
    reason: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct WebIssueItem {
    #[serde(flatten)]
    item: crate::domain::IssueListItem,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait: Option<WebWait>,
    unresolved_decision_count: usize,
    decision_questions: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct WebStatus {
    attention: Vec<WebIssueItem>,
    stale: Vec<WebIssueItem>,
    blocked: Vec<WebIssueItem>,
    recently_completed: Vec<WebIssueItem>,
    active: Vec<WebIssueItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct WebIssueDetail {
    project: String,
    issue: crate::domain::Issue,
    history: Vec<crate::domain::DomainEvent>,
    decisions: Vec<crate::domain::DecisionRequest>,
    dependencies: Vec<crate::domain::IssueDependency>,
    worktrees: Vec<crate::domain::IssueWorktree>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait: Option<WebWait>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveDecisionRequest {
    expected_revision: i64,
    answer: String,
    next_state: crate::domain::IssueState,
    summary: Option<String>,
    verification: Option<String>,
    reason: Option<String>,
    wait_kind: Option<crate::domain::WaitKind>,
}

fn read_request(stream: &mut std::net::TcpStream) -> Result<Request, crate::error::AppError> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| {
            crate::error::AppError::Internal(format!("could not configure web request: {error}"))
        })?;
    let mut reader = std::io::BufReader::new(stream);
    let mut request_line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut request_line).map_err(|error| {
        crate::error::AppError::Internal(format!("could not read web request: {error}"))
    })?;
    if request_line.len() > MAX_REQUEST_LINE {
        return Err(crate::error::AppError::InvalidInput(
            "web request line is too long".to_owned(),
        ));
    }
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| crate::error::AppError::InvalidInput("web request is empty".to_owned()))?;
    let target = parts.next().ok_or_else(|| {
        crate::error::AppError::InvalidInput("web request has no target".to_owned())
    })?;
    let version = parts.next().ok_or_else(|| {
        crate::error::AppError::InvalidInput("web request has no HTTP version".to_owned())
    })?;
    if version != "HTTP/1.0" && version != "HTTP/1.1" {
        return Err(crate::error::AppError::InvalidInput(
            "web request uses an unsupported HTTP version".to_owned(),
        ));
    }
    let mut headers_terminated = false;
    let mut content_length = None;
    for _ in 0..MAX_HEADERS {
        let mut header = String::new();
        let read = std::io::BufRead::read_line(&mut reader, &mut header).map_err(|error| {
            crate::error::AppError::Internal(format!("could not read web headers: {error}"))
        })?;
        if read == 0 || header == "\r\n" || header == "\n" {
            headers_terminated = true;
            break;
        }
        let (name, value) = header.split_once(':').ok_or_else(|| {
            crate::error::AppError::InvalidInput("web request has an invalid header".to_owned())
        })?;
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err(crate::error::AppError::InvalidInput(
                        "web request has duplicate content-length headers".to_owned(),
                    ));
                }
                let length = value.trim().parse::<usize>().map_err(|_| {
                    crate::error::AppError::InvalidInput(
                        "web request content-length is invalid".to_owned(),
                    )
                })?;
                if length > MAX_REQUEST_BODY {
                    return Err(crate::error::AppError::InvalidInput(
                        "web request body is too large".to_owned(),
                    ));
                }
                content_length = Some(length);
            }
            "transfer-encoding" => {
                return Err(crate::error::AppError::InvalidInput(
                    "web request transfer-encoding is unsupported".to_owned(),
                ));
            }
            _ => {}
        }
    }
    if !headers_terminated {
        return Err(crate::error::AppError::InvalidInput(
            "web request has too many headers".to_owned(),
        ));
    }

    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let path = percent_decode(raw_path)?;
    let query = parse_query(raw_query)?;
    let mut body = vec![0; content_length.unwrap_or(0)];
    if !body.is_empty() {
        std::io::Read::read_exact(&mut reader, &mut body).map_err(|error| {
            crate::error::AppError::InvalidInput(format!(
                "could not read web request body: {error}"
            ))
        })?;
    }
    Ok(Request {
        method: method.to_owned(),
        path,
        query,
        body,
    })
}

fn route_request(database_path: &std::path::Path, request: &Request) -> HttpResponse {
    if request.method == "POST" && request.path.starts_with("/api/decisions/") {
        return resolve_decision_route(database_path, request);
    }
    if request.method != "GET" {
        return error_response(
            405,
            crate::error::AppError::InvalidInput(
                "web UI only supports GET and decision resolution POST".to_owned(),
            ),
        );
    }

    match request.path.as_str() {
        "/" => HttpResponse::text(200, "text/html; charset=utf-8", INDEX_HTML),
        "/app.css" => HttpResponse::text(200, "text/css; charset=utf-8", APP_CSS),
        "/state.js" => HttpResponse::text(200, "text/javascript; charset=utf-8", STATE_JS),
        "/app.js" => HttpResponse::text(200, "text/javascript; charset=utf-8", APP_JS),
        "/api/status" => with_read_database(database_path, |app| web_status(app).map(json_success)),
        "/api/projects" => {
            with_read_database(database_path, |app| app.list_projects().map(json_success))
        }
        "/api/issues" => with_read_database(database_path, |app| {
            let filter = issue_filter(&request.query)?;
            let issues = app.list_issues(&filter)?;
            issues
                .into_iter()
                .map(|item| web_issue_item(app, item))
                .collect::<Result<Vec<_>, _>>()
                .map(json_success)
        }),
        path if path.starts_with("/api/issues/") => {
            let number = path
                .strip_prefix("/api/issues/")
                .and_then(|value| value.parse::<i64>().ok());
            let Some(number) = number else {
                return error_response(
                    400,
                    crate::error::AppError::InvalidInput(
                        "issue number must be a positive integer".to_owned(),
                    ),
                );
            };
            let Some(project) = request.query.get("project") else {
                return error_response(
                    400,
                    crate::error::AppError::InvalidInput(
                        "issue detail requires a project query parameter".to_owned(),
                    ),
                );
            };
            with_read_database(database_path, |app| {
                let issue = app.show_issue(project, number)?;
                let history = app.issue_history(project, number)?;
                let decisions = app.list_decisions(project, number)?;
                let dependencies = app.list_dependencies(&format!("{project}#{number}"), None)?;
                let worktrees = app.list_worktrees(&format!("{project}#{number}"), None)?;
                let item = crate::domain::IssueListItem {
                    project: project.clone(),
                    issue: issue.clone(),
                    worktrees: worktrees
                        .iter()
                        .filter(|worktree| worktree.active)
                        .cloned()
                        .collect(),
                };
                let wait = web_wait_context(app, &item, &decisions)?;
                Ok(json_success(WebIssueDetail {
                    project: project.clone(),
                    issue,
                    history,
                    decisions,
                    dependencies,
                    worktrees,
                    wait,
                }))
            })
        }
        _ => error_response(
            404,
            crate::error::AppError::NotFound("web route not found".to_owned()),
        ),
    }
}

fn resolve_decision_route(database_path: &std::path::Path, request: &Request) -> HttpResponse {
    let request_id = request
        .path
        .strip_prefix("/api/decisions/")
        .and_then(|path| path.strip_suffix("/resolve"))
        .filter(|value| !value.is_empty() && !value.contains('/'));
    let Some(request_id) = request_id else {
        return error_response(
            404,
            crate::error::AppError::NotFound("web route not found".to_owned()),
        );
    };
    let request_id = match uuid::Uuid::parse_str(request_id) {
        Ok(request_id) => request_id,
        Err(_) => {
            return error_response(
                400,
                crate::error::AppError::InvalidInput(
                    "decision request id must be a UUID".to_owned(),
                ),
            );
        }
    };
    let payload: ResolveDecisionRequest = match serde_json::from_slice(&request.body) {
        Ok(payload) => payload,
        Err(_) => {
            return error_response(
                400,
                crate::error::AppError::InvalidInput(
                    "decision resolution body must be valid JSON".to_owned(),
                ),
            );
        }
    };
    if payload.next_state == crate::domain::IssueState::InProgress {
        return error_response(
            400,
            crate::error::AppError::InvalidInput(
                "decision resolution next_state must be todo, blocked, done, or cancelled"
                    .to_owned(),
            ),
        );
    }
    with_database(database_path, |app| {
        let context = crate::domain::ExecutionContext::resolve()?;
        let resolution = crate::domain::DecisionResolutionInput::new(
            payload.next_state,
            payload.summary,
            payload.verification,
            payload.reason,
            payload.wait_kind,
        );
        app.resolve_decision(
            &request_id.to_string(),
            &payload.answer,
            Some(payload.expected_revision),
            resolution,
            &context,
        )
        .map(json_success)
    })
}

fn web_status(app: &mut crate::app::App) -> Result<WebStatus, crate::error::AppError> {
    let status = app.status()?;
    Ok(WebStatus {
        attention: web_issue_items(app, status.attention)?,
        stale: web_issue_items(app, status.stale)?,
        blocked: web_issue_items(app, status.blocked)?,
        recently_completed: web_issue_items(app, status.recently_completed)?,
        active: web_issue_items(app, status.active)?,
    })
}

fn web_issue_items(
    app: &mut crate::app::App,
    items: Vec<crate::domain::IssueListItem>,
) -> Result<Vec<WebIssueItem>, crate::error::AppError> {
    items
        .into_iter()
        .map(|item| web_issue_item(app, item))
        .collect()
}

fn web_issue_item(
    app: &mut crate::app::App,
    item: crate::domain::IssueListItem,
) -> Result<WebIssueItem, crate::error::AppError> {
    let decisions = app.list_decisions(&item.project, item.issue.number)?;
    let unresolved_decisions = decisions
        .iter()
        .filter(|decision| decision.status == "open")
        .collect::<Vec<_>>();
    let wait = web_wait_context(app, &item, &decisions)?;
    Ok(WebIssueItem {
        item,
        wait,
        unresolved_decision_count: unresolved_decisions.len(),
        decision_questions: unresolved_decisions
            .into_iter()
            .map(|decision| decision.question.clone())
            .collect(),
    })
}

fn web_wait_context(
    app: &mut crate::app::App,
    item: &crate::domain::IssueListItem,
    decisions: &[crate::domain::DecisionRequest],
) -> Result<Option<WebWait>, crate::error::AppError> {
    if decisions.iter().any(|decision| decision.status == "open") {
        return Ok(Some(WebWait {
            kind: Some(crate::domain::WaitKind::Human),
            label: wait_kind_label(crate::domain::WaitKind::Human).to_owned(),
            reason: "A human decision is required".to_owned(),
        }));
    }
    if item.issue.state != crate::domain::IssueState::Blocked {
        return Ok(None);
    }

    let history = app.issue_history(&item.project, item.issue.number)?;
    if let Some(event) = history
        .iter()
        .rev()
        .find(|event| event.event_type == "issue_blocked")
    {
        let kind = event
            .metadata
            .get("wait_kind")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        let reason = event
            .metadata
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .filter(|reason| !reason.trim().is_empty())
            .unwrap_or("Waiting details are not recorded")
            .to_owned();
        return Ok(Some(WebWait {
            label: kind
                .map(wait_kind_label)
                .unwrap_or("Waiting reason")
                .to_owned(),
            kind,
            reason,
        }));
    }

    let reference = format!("{}#{}", item.project, item.issue.number);
    let dependencies = app.list_dependencies(&reference, None)?;
    if dependencies
        .iter()
        .any(|dependency| dependency.blocked == reference)
    {
        return Ok(Some(WebWait {
            kind: Some(crate::domain::WaitKind::Dependency),
            label: wait_kind_label(crate::domain::WaitKind::Dependency).to_owned(),
            reason: "Waiting for a blocking dependency".to_owned(),
        }));
    }

    Ok(Some(WebWait {
        kind: None,
        label: "Waiting reason".to_owned(),
        reason: "Waiting details are not recorded".to_owned(),
    }))
}

fn wait_kind_label(kind: crate::domain::WaitKind) -> &'static str {
    match kind {
        crate::domain::WaitKind::Human => "Human decision",
        crate::domain::WaitKind::Dependency => "Blocking dependency",
        crate::domain::WaitKind::External => "External system",
    }
}

fn with_database<F>(database_path: &std::path::Path, operation: F) -> HttpResponse
where
    F: FnOnce(&mut crate::app::App) -> Result<HttpResponse, crate::error::AppError>,
{
    match crate::store::Database::open(database_path) {
        Ok(database) => operation(&mut crate::app::App::new(database))
            .unwrap_or_else(|error| error_response(status_code(&error), error)),
        Err(error) => error_response(status_code(&error), error),
    }
}

fn with_read_database<F>(database_path: &std::path::Path, operation: F) -> HttpResponse
where
    F: FnOnce(&mut crate::app::App) -> Result<HttpResponse, crate::error::AppError>,
{
    match crate::store::Database::open_for_web_read(database_path) {
        Ok(database) => operation(&mut crate::app::App::new(database))
            .unwrap_or_else(|error| error_response(status_code(&error), error)),
        Err(error) => error_response(status_code(&error), error),
    }
}

fn issue_filter(
    query: &std::collections::BTreeMap<String, String>,
) -> Result<crate::domain::IssueFilter, crate::error::AppError> {
    let project = query.get("project").cloned();
    let include_done = query
        .get("include_done")
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    Ok(crate::domain::IssueFilter {
        projects: project.into_iter().collect(),
        states: Vec::new(),
        priorities: Vec::new(),
        assignee: None,
        updated_after: None,
        query: query.get("query").cloned(),
        include_done,
    })
}

fn parse_query(
    raw_query: &str,
) -> Result<std::collections::BTreeMap<String, String>, crate::error::AppError> {
    let mut query = std::collections::BTreeMap::new();
    for pair in raw_query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(
            percent_decode(key)?,
            percent_decode(value.replace('+', " ").as_str())?,
        );
    }
    Ok(query)
}

fn percent_decode(value: &str) -> Result<String, crate::error::AppError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(crate::error::AppError::InvalidInput(
                    "web URL contains an incomplete escape".to_owned(),
                ));
            }
            let high = hex_value(bytes[index + 1]).ok_or_else(|| {
                crate::error::AppError::InvalidInput(
                    "web URL contains an invalid escape".to_owned(),
                )
            })?;
            let low = hex_value(bytes[index + 2]).ok_or_else(|| {
                crate::error::AppError::InvalidInput(
                    "web URL contains an invalid escape".to_owned(),
                )
            })?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| crate::error::AppError::InvalidInput("web URL is not valid UTF-8".to_owned()))
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn json_success(data: impl serde::Serialize) -> HttpResponse {
    let body = serde_json::json!({ "schema_version": 1, "data": data });
    HttpResponse::json(200, &body)
}

fn error_response(status: u16, error: crate::error::AppError) -> HttpResponse {
    let body = serde_json::json!({
        "schema_version": 1,
        "error": { "code": error.code(), "message": error.to_string() }
    });
    HttpResponse::json(status, &body)
}

fn status_code(error: &crate::error::AppError) -> u16 {
    match error {
        crate::error::AppError::InvalidInput(_)
        | crate::error::AppError::InvalidBackup(_)
        | crate::error::AppError::BackupConfirmationRequired
        | crate::error::AppError::UnsupportedDatabaseSchemaVersion { .. } => 400,
        crate::error::AppError::NotFound(_) | crate::error::AppError::DatabaseNotInitialized => 404,
        crate::error::AppError::DatabaseBusy(_) => 503,
        crate::error::AppError::Conflict(_)
        | crate::error::AppError::IdempotencyConflict
        | crate::error::AppError::InvalidTransition(_)
        | crate::error::AppError::RevisionConflict { .. }
        | crate::error::AppError::ProjectNameConflict
        | crate::error::AppError::BackupOutputExists
        | crate::error::AppError::BackupDestinationInUse => 409,
        crate::error::AppError::Internal(_)
        | crate::error::AppError::DatabaseAlreadyInitialized
        | crate::error::AppError::AuditIntegrity { .. }
        | crate::error::AppError::AuditOperation { .. }
        | crate::error::AppError::BackupOperation { .. }
        | crate::error::AppError::SelfUpdateFailed(_)
        | crate::error::AppError::SelfUninstallFailed(_) => 500,
    }
}

struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    fn text(status: u16, content_type: &'static str, body: &str) -> Self {
        Self {
            status,
            content_type,
            body: body.as_bytes().to_vec(),
        }
    }

    fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body: body.to_string().into_bytes(),
        }
    }
}

fn write_response(
    stream: &mut std::net::TcpStream,
    response: &HttpResponse,
) -> Result<(), crate::error::AppError> {
    use std::io::Write as _;
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )
    .and_then(|()| stream.write_all(&response.body))
    .map_err(|error| crate::error::AppError::Internal(format!("could not write web response: {error}")))
}

#[cfg(test)]
mod tests {
    use super::status_code;

    #[test]
    fn database_busy_is_exposed_as_service_unavailable() {
        assert_eq!(
            status_code(&crate::error::AppError::DatabaseBusy(
                "database is busy".to_owned()
            )),
            503
        );
    }
}
