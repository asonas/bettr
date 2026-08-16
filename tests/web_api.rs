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

    fn request(&self, method: &str, path: &str) -> (u16, String) {
        Self::request_at(&self.address, method, path)
    }

    fn request_at(address: &str, method: &str, path: &str) -> (u16, String) {
        let address = address.trim_start_matches("http://");
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
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
    assert_eq!(issue["data"]["history"][1]["event_type"], "comment_added");
    assert_eq!(
        issue["data"]["history"][1]["metadata"]["body"],
        "The local view should preserve context."
    );
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
            std::thread::spawn(move || WebProcess::request_at(&address, "GET", "/api/status"))
        })
        .collect::<Vec<_>>();
    for request in requests {
        let (status, body) = request.join().unwrap();
        assert_eq!(status, 200);
        let response: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(response["schema_version"], 1);
    }
}
