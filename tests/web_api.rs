mod support;

use std::io::{BufRead as _, Read as _, Write as _};
use std::net::TcpStream;
use std::process::{Child, Stdio};
use std::time::Duration;

struct WebProcess {
    child: Option<Child>,
    address: String,
}

impl WebProcess {
    fn start(app: &support::TestApp) -> Self {
        let binary = assert_cmd::cargo::cargo_bin!("bettr");
        let mut command = std::process::Command::new(binary);
        command
            .args([
                "--database",
                app.database.to_str().unwrap(),
                "web",
                "--port",
                "0",
            ])
            .env_clear()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stdout = std::io::BufReader::new(stdout);
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert!(line.starts_with("listening http://127.0.0.1:"), "{line}");
        Self {
            address: line.trim().trim_start_matches("listening ").to_owned(),
            child: Some(child),
        }
    }

    fn get(&self, path: &str) -> (u16, String) {
        self.request("GET", path)
    }

    fn post_json(&self, path: &str, body: &serde_json::Value) -> (u16, String) {
        let body = body.to_string();
        self.request_with_body("POST", path, &body)
    }

    fn request(&self, method: &str, path: &str) -> (u16, String) {
        self.request_with_body(method, path, "")
    }

    fn request_with_body(&self, method: &str, path: &str, body: &str) -> (u16, String) {
        Self::request_at(&self.address, method, path, body)
    }

    fn request_at(address: &str, method: &str, path: &str, body: &str) -> (u16, String) {
        let address = address.trim_start_matches("http://");
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (status, body.to_owned())
    }
}

fn request_decision(app: &support::TestApp, question: &str, session: &str) -> serde_json::Value {
    let output = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", session)
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The implementation has two compatible choices.",
            "--question",
            question,
            "--option",
            "Use option A.",
            "--option",
            "Use option B.",
            "--recommendation",
            "Use option A.",
            "--resume-condition",
            "The selected option is implemented and verified.",
            "--background",
            "The implementation has two compatible choices.",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

impl Drop for WebProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn initialize_app() -> support::TestApp {
    let app = support::TestApp::new();
    app.command().args(["init"]).assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Review the supervisor view",
            "--priority",
            "high",
        ])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "The local view should preserve context.",
        ])
        .assert()
        .success();
    app
}

#[test]
fn web_serves_status_and_embedded_assets() {
    let app = initialize_app();
    let server = WebProcess::start(&app);

    let (status, body) = server.get("/api/status");
    assert_eq!(status, 200);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["schema_version"], 1);
    assert_eq!(
        response["data"]["active"][0]["title"],
        "Review the supervisor view"
    );

    let (status, body) = server.get("/");
    assert_eq!(status, 200);
    assert!(body.contains("bettr"));
    assert!(body.contains("skip-link"));
    assert!(body.contains("id=\"main\""));
    assert!(body.contains("aria-live=\"polite\""));
    assert!(body.contains("id=\"updated-nav\""));
    assert!(body.contains("id=\"updated-menu\""));
    assert!(!body.contains("id=\"update-banner\""));
    assert!(!body.contains("id=\"search-nav\""));
    assert!(!body.contains("data-nav=\"projects\""));
    assert!(!body.contains("class=\"project-nav-label\""));
    assert!(body.contains("data-nav=\"recent\""));
    assert!(body.contains("id=\"project-nav-list\""));
    assert!(body.contains("<script type=\"module\" src=\"/app.js\"></script>"));

    let (status, body) = server.get("/app.css");
    assert_eq!(status, 200);
    assert!(body.contains("--surface"));
    assert!(body.contains(":focus-visible"));
    assert!(body.contains("prefers-reduced-motion"));
    assert!(body.contains("property-rail"));

    let (status, body) = server.get("/app.js");
    assert_eq!(status, 200);
    assert!(body.contains("export function createWebController"));
    assert!(body.contains("import { allIssues, applyStatusUpdate, kanbanColumns }"));

    let (status, body) = server.get("/state.js");
    assert_eq!(status, 200);
    assert!(body.contains("export { kanbanColumns"));
}

#[test]
fn web_page_headings_use_readable_tracking() {
    let app_css = include_str!("../src/web/app.css");

    assert!(app_css.contains("font-weight: 720; letter-spacing: -.02em; line-height: 1.1;"));
    assert!(!app_css.contains("letter-spacing: -.045em"));
}

#[test]
fn web_does_not_advertise_or_handle_keyboard_shortcuts() {
    let index_html = include_str!("../src/web/index.html");
    let app_css = include_str!("../src/web/app.css");

    assert!(!index_html.contains("<kbd>"));
    assert!(!index_html.contains("Supervision"));
    assert!(!app_css.contains(".nav-list kbd"));
}

#[test]
fn web_serves_project_issue_detail_with_activity() {
    let app = initialize_app();
    let server = WebProcess::start(&app);

    let (status, body) = server.get("/api/projects");
    assert_eq!(status, 200);
    let projects: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(projects["data"][0]["name"], "bettr");

    let (status, body) = server.get("/api/issues/1?project=bettr");
    assert_eq!(status, 200);
    let issue: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(issue["data"]["issue"]["number"], 1);
    assert_eq!(issue["data"]["dependencies"], serde_json::json!([]));
    assert_eq!(issue["data"]["worktrees"], serde_json::json!([]));
    assert_eq!(issue["data"]["history"][1]["event_type"], "comment_added");
    assert_eq!(
        issue["data"]["history"][1]["metadata"]["body"],
        "The local view should preserve context."
    );
}

#[test]
fn web_read_endpoints_do_not_append_audit_events() {
    let app = initialize_app();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let before: i64 = connection
        .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
        .unwrap();
    drop(connection);
    let server = WebProcess::start(&app);

    for path in [
        "/api/status",
        "/api/projects",
        "/api/issues/1?project=bettr",
    ] {
        let (status, body) = server.get(path);
        assert_eq!(status, 200, "{path}: {body}");
    }

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let after: i64 = connection
        .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn web_exposes_waiting_context_and_multiple_decisions() {
    let app = initialize_app();
    let first = request_decision(&app, "Which parser should be used?", "session-a");
    request_decision(&app, "Which rollout should be used?", "session-b");
    let server = WebProcess::start(&app);

    let (status, body) = server.get("/api/status");
    assert_eq!(status, 200);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        response["data"]["attention"][0]["unresolved_decision_count"],
        2
    );
    assert_eq!(
        response["data"]["attention"][0]["wait"]["label"],
        "Human decision"
    );
    assert_eq!(
        response["data"]["attention"][0]["decision_questions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let (status, body) = server.get("/api/issues?project=bettr&include_done=true");
    assert_eq!(status, 200);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        response["data"][0]["wait"]["reason"],
        "A human decision is required"
    );

    let (status, body) = server.get("/api/issues/1?project=bettr");
    assert_eq!(status, 200);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["data"]["decisions"].as_array().unwrap().len(), 2);
    assert_eq!(response["data"]["decisions"][0]["id"], first["id"]);
    assert_eq!(
        response["data"]["decisions"][0]["blocker"],
        "The implementation has two compatible choices."
    );
    assert_eq!(
        response["data"]["decisions"][0]["options"][0],
        "Use option A."
    );
    assert_eq!(
        response["data"]["decisions"][0]["recommendation"],
        "Use option A."
    );
    assert_eq!(
        response["data"]["decisions"][0]["resume_condition"],
        "The selected option is implemented and verified."
    );
    assert_eq!(response["data"]["wait"]["label"], "Human decision");
    assert!(
        response["data"]["wait"]["reason"]
            .as_str()
            .unwrap()
            .contains("human")
    );
}

#[test]
fn web_exposes_existing_block_reason_and_wait_kind() {
    let app = initialize_app();
    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "issue",
            "start",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--json",
        ])
        .assert()
        .success();
    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "issue",
            "block",
            "1",
            "--project",
            "bettr",
            "--revision",
            "2",
            "--reason",
            "Waiting for the deployment window.",
            "--wait-kind",
            "external",
            "--json",
        ])
        .assert()
        .success();
    let server = WebProcess::start(&app);

    let (status, body) = server.get("/api/status");
    assert_eq!(status, 200);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["data"]["blocked"][0]["wait"]["kind"], "external");
    assert_eq!(
        response["data"]["blocked"][0]["wait"]["label"],
        "External system"
    );
    assert_eq!(
        response["data"]["blocked"][0]["wait"]["reason"],
        "Waiting for the deployment window."
    );
}

#[test]
fn web_resolves_a_decision_with_the_existing_human_contract() {
    let app = initialize_app();
    let request = request_decision(&app, "Which parser should be used?", "session-a");
    let server = WebProcess::start(&app);

    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    let revision = detail["data"]["issue"]["revision"].as_i64().unwrap();
    let request_id = request["id"].as_str().unwrap();
    let (status, body) = server.post_json(
        &format!("/api/decisions/{request_id}/resolve"),
        &serde_json::json!({
            "expected_revision": revision,
            "answer": "Use the streaming parser.",
            "next_state": "todo"
        }),
    );

    assert_eq!(status, 200);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["data"]["status"], "resolved");
    assert_eq!(response["data"]["resolver_kind"], "human");

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                 WHERE operation = 'decision_resolve' AND success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    let (status, body) = server.get("/api/issues/1?project=bettr");
    assert_eq!(status, 200);
    let detail: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(detail["data"]["issue"]["state"], "todo");
    assert_eq!(detail["data"]["decisions"][0]["status"], "resolved");
    assert_eq!(
        detail["data"]["history"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["event_type"],
        "decision_resolved"
    );
}

#[test]
fn web_rejects_a_stale_revision_without_resolving_the_decision() {
    let app = initialize_app();
    let request = request_decision(&app, "Which parser should be used?", "session-a");
    let server = WebProcess::start(&app);

    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    let revision = detail["data"]["issue"]["revision"].as_i64().unwrap();
    app.command()
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            &revision.to_string(),
            "--title",
            "Changed while reviewing",
        ])
        .assert()
        .success();

    let (status, body) = server.post_json(
        &format!("/api/decisions/{}/resolve", request["id"].as_str().unwrap()),
        &serde_json::json!({
            "expected_revision": revision,
            "answer": "Use option A.",
            "next_state": "todo"
        }),
    );

    assert_eq!(status, 409);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], "revision_conflict");
    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    assert_eq!(detail["data"]["issue"]["state"], "blocked");
    assert_eq!(detail["data"]["decisions"][0]["status"], "open");
}

#[test]
fn web_keeps_done_resolution_blocked_when_another_decision_is_open() {
    let app = initialize_app();
    let first = request_decision(&app, "Which parser should be used?", "session-a");
    request_decision(&app, "Which rollout should be used?", "session-b");
    let server = WebProcess::start(&app);

    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    let revision = detail["data"]["issue"]["revision"].as_i64().unwrap();
    let (status, body) = server.post_json(
        &format!("/api/decisions/{}/resolve", first["id"].as_str().unwrap()),
        &serde_json::json!({
            "expected_revision": revision,
            "answer": "Use option A.",
            "next_state": "done",
            "summary": "Selected option A.",
            "verification": "Reviewed the integration tests."
        }),
    );

    assert_eq!(status, 409);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], "conflict");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("another unresolved")
    );
    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    assert_eq!(detail["data"]["issue"]["state"], "blocked");
    assert_eq!(detail["data"]["decisions"][0]["status"], "open");
}

#[test]
fn web_rejects_invalid_resolution_input_without_mutating_the_decision() {
    let app = initialize_app();
    let request = request_decision(&app, "Which parser should be used?", "session-a");
    let server = WebProcess::start(&app);
    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    let revision = detail["data"]["issue"]["revision"].as_i64().unwrap();
    let path = format!("/api/decisions/{}/resolve", request["id"].as_str().unwrap());

    for body in [
        serde_json::json!({"answer": "Use option A.", "next_state": "todo"}),
        serde_json::json!({
            "expected_revision": revision,
            "answer": "Use option A.",
            "next_state": "todo",
            "unexpected": true
        }),
        serde_json::json!({
            "expected_revision": revision,
            "answer": "Use option A.",
            "next_state": "in_progress"
        }),
        serde_json::json!({
            "expected_revision": revision,
            "answer": "Use option A.",
            "next_state": "blocked"
        }),
        serde_json::json!({
            "expected_revision": revision,
            "answer": "Use option A.",
            "next_state": "done"
        }),
        serde_json::json!({
            "expected_revision": revision,
            "answer": "Use option A.",
            "next_state": "cancelled"
        }),
    ] {
        let (status, response_body) = server.post_json(&path, &body);
        assert_eq!(status, 400, "{response_body}");
        let response: serde_json::Value = serde_json::from_str(&response_body).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
    }

    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    assert_eq!(detail["data"]["issue"]["revision"], revision);
    assert_eq!(detail["data"]["issue"]["state"], "blocked");
    assert_eq!(detail["data"]["decisions"][0]["status"], "open");
}

#[test]
fn web_treats_a_second_submission_as_a_conflict_without_replaying_it() {
    let app = initialize_app();
    let request = request_decision(&app, "Which parser should be used?", "session-a");
    let server = WebProcess::start(&app);
    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    let revision = detail["data"]["issue"]["revision"].as_i64().unwrap();
    let path = format!("/api/decisions/{}/resolve", request["id"].as_str().unwrap());
    let body = serde_json::json!({
        "expected_revision": revision,
        "answer": "Use option A.",
        "next_state": "todo"
    });

    let (status, _) = server.post_json(&path, &body);
    assert_eq!(status, 200);
    let (status, response_body) = server.post_json(&path, &body);
    assert_eq!(status, 409);
    let response: serde_json::Value = serde_json::from_str(&response_body).unwrap();
    assert_eq!(response["error"]["code"], "conflict");

    let (_, detail_body) = server.get("/api/issues/1?project=bettr");
    let detail: serde_json::Value = serde_json::from_str(&detail_body).unwrap();
    assert_eq!(detail["data"]["issue"]["revision"], revision + 1);
    assert_eq!(detail["data"]["issue"]["state"], "todo");
    assert_eq!(detail["data"]["decisions"][0]["status"], "resolved");
}

#[test]
fn web_returns_json_not_found_for_unknown_routes() {
    let app = initialize_app();
    let server = WebProcess::start(&app);

    let (status, body) = server.get("/missing");
    assert_eq!(status, 404);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], "not_found");
}

#[test]
fn web_filters_issue_list_by_project_and_query() {
    let app = initialize_app();
    let server = WebProcess::start(&app);

    let (status, body) = server.get("/api/issues?project=bettr&query=supervisor&include_done=true");
    assert_eq!(status, 200);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["data"].as_array().unwrap().len(), 1);
    assert_eq!(response["data"][0]["project"], "bettr");

    let (status, body) = server.get("/api/issues/1");
    assert_eq!(status, 400);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], "invalid_input");
}

#[test]
fn web_rejects_non_get_requests() {
    let app = initialize_app();
    let server = WebProcess::start(&app);

    let (status, body) = server.request("POST", "/api/status");
    assert_eq!(status, 405);
    let response: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(response["error"]["code"], "invalid_input");
}

#[test]
fn web_serves_concurrent_read_requests() {
    let app = initialize_app();
    let server = WebProcess::start(&app);
    let address = server.address.clone();
    let requests = (0..4)
        .map(|_| {
            let address = address.clone();
            std::thread::spawn(move || WebProcess::request_at(&address, "GET", "/api/status", ""))
        })
        .collect::<Vec<_>>();
    for request in requests {
        let (status, body) = request.join().unwrap();
        assert_eq!(status, 200);
        let response: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(response["schema_version"], 1);
    }
}
