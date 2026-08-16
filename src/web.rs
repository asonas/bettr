const INDEX_HTML: &str = include_str!("web/index.html");
const APP_CSS: &str = include_str!("web/app.css");
const APP_JS: &str = include_str!("web/app.js");

const MAX_REQUEST_LINE: usize = 8 * 1024;
const MAX_HEADERS: usize = 64;

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
    for _ in 0..MAX_HEADERS {
        let mut header = String::new();
        let read = std::io::BufRead::read_line(&mut reader, &mut header).map_err(|error| {
            crate::error::AppError::Internal(format!("could not read web headers: {error}"))
        })?;
        if read == 0 || header == "\r\n" || header == "\n" {
            headers_terminated = true;
            break;
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
    Ok(Request {
        method: method.to_owned(),
        path,
        query,
    })
}

fn route_request(database_path: &std::path::Path, request: &Request) -> HttpResponse {
    if request.method != "GET" {
        return error_response(
            405,
            crate::error::AppError::InvalidInput("web UI only supports GET".to_owned()),
        );
    }

    match request.path.as_str() {
        "/" => HttpResponse::text(200, "text/html; charset=utf-8", INDEX_HTML),
        "/app.css" => HttpResponse::text(200, "text/css; charset=utf-8", APP_CSS),
        "/app.js" => HttpResponse::text(200, "text/javascript; charset=utf-8", APP_JS),
        "/api/status" => with_database(database_path, |app| app.status().map(json_success)),
        "/api/projects" => {
            with_database(database_path, |app| app.list_projects().map(json_success))
        }
        "/api/issues" => with_database(database_path, |app| {
            let filter = issue_filter(&request.query)?;
            app.list_issues(&filter).map(json_success)
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
            with_database(database_path, |app| {
                let issue = app.show_issue(project, number)?;
                let history = app.issue_history(project, number)?;
                Ok(json_success(serde_json::json!({
                    "project": project,
                    "issue": issue,
                    "history": history,
                })))
            })
        }
        _ => error_response(
            404,
            crate::error::AppError::NotFound("web route not found".to_owned()),
        ),
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
        crate::error::AppError::InvalidInput(_) => 400,
        crate::error::AppError::NotFound(_) | crate::error::AppError::DatabaseNotInitialized => 404,
        crate::error::AppError::DatabaseBusy(_) => 503,
        crate::error::AppError::Conflict(_)
        | crate::error::AppError::InvalidTransition(_)
        | crate::error::AppError::RevisionConflict { .. }
        | crate::error::AppError::ProjectNameConflict => 409,
        crate::error::AppError::Internal(_)
        | crate::error::AppError::DatabaseAlreadyInitialized => 500,
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
