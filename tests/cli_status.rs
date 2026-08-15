mod support;

fn create_project(app: &crate::support::TestApp, name: &str) {
    app.command()
        .args(["project", "create", name])
        .assert()
        .success();
}

fn create_issue(app: &crate::support::TestApp, project: &str, title: &str) -> String {
    let output = app
        .command()
        .args([
            "issue",
            "create",
            "--project",
            project,
            "--title",
            title,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn set_state(app: &crate::support::TestApp, id: &str, state: &str, updated_at: &str) {
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute(
            "UPDATE issues SET state = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![state, updated_at, id],
        )
        .unwrap();
}

fn seeded_app() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");
    create_project(&app, "beta");

    let blocked = create_issue(&app, "alpha", "Waiting on access");
    let done = create_issue(&app, "alpha", "Shipped parser");
    let cancelled = create_issue(&app, "beta", "Dropped experiment");
    let progress = create_issue(&app, "beta", "Build renderer");
    create_issue(&app, "alpha", "Write documentation");

    set_state(&app, &blocked, "blocked", "2026-08-15T01:00:00Z");
    set_state(&app, &done, "done", "2026-08-15T02:00:00Z");
    set_state(&app, &cancelled, "cancelled", "2026-08-15T03:00:00Z");
    set_state(&app, &progress, "in_progress", "2026-08-15T04:00:00Z");
    app
}

#[test]
fn status_json_groups_all_projects_and_keeps_phase_one_empty_arrays() {
    let app = seeded_app();
    let output = app.command().args(["status", "--json"]).output().unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    let status = &response["data"];
    assert_eq!(status["attention"], serde_json::json!([]));
    assert_eq!(status["stale"], serde_json::json!([]));
    assert_eq!(status["blocked"].as_array().unwrap().len(), 1);
    assert_eq!(status["blocked"][0]["project"], "alpha");
    assert_eq!(status["blocked"][0]["title"], "Waiting on access");
    assert_eq!(status["recently_completed"].as_array().unwrap().len(), 2);
    assert_eq!(status["active"].as_array().unwrap().len(), 2);
    assert_eq!(status["active"][0]["project"], "beta");
    assert_eq!(status["active"][0]["state"], "in_progress");

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                 WHERE operation = 'status' AND success = 1 AND metadata_json = '{}'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn status_human_omits_empty_sections_and_qualifies_issue_references() {
    let app = seeded_app();
    let output = app.command().arg("status").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("attention"));
    assert!(!stdout.contains("stale"));
    let blocked = stdout.find("blocked").unwrap();
    let completed = stdout.find("recently completed").unwrap();
    let active = stdout.find("active").unwrap();
    assert!(blocked < completed);
    assert!(completed < active);
    assert!(stdout.contains("alpha#1"));
    assert!(stdout.contains("beta#1"));
}
