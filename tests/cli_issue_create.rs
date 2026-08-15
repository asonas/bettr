mod support;

#[test]
fn issue_create_returns_a_project_local_issue_in_the_json_envelope() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    let output = app
        .command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Build local core",
            "--body",
            "First vertical slice",
            "--priority",
            "high",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    let issue = &response["data"];
    let id = issue["id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(id.chars().filter(|character| *character == '-').count(), 4);
    assert_eq!(issue["number"], 1);
    assert_eq!(issue["title"], "Build local core");
    assert_eq!(issue["body"], "First vertical slice");
    assert_eq!(issue["state"], "todo");
    assert_eq!(issue["priority"], "high");
    assert!(issue["assignee_kind"].is_null());
    assert!(issue["assignee_name"].is_null());
    assert_eq!(issue["revision"], 1);
    assert!(issue["created_at"].as_str().unwrap().ends_with('Z'));
    assert!(issue["updated_at"].as_str().unwrap().ends_with('Z'));

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT event_type FROM domain_events WHERE issue_id = ?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "issue_created"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT target_id FROM audit_events
                 WHERE operation = 'issue_create' AND success = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        id
    );
}

#[test]
fn issue_create_allocates_numbers_per_project() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    for expected_number in [1, 2] {
        let output = app
            .command()
            .args([
                "issue",
                "create",
                "--project",
                "bettr",
                "--title",
                "Build local core",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["data"]["number"], expected_number);
    }

    app.command()
        .args(["project", "create", "other"])
        .assert()
        .success();
    let output = app
        .command()
        .args([
            "issue",
            "create",
            "--project",
            "other",
            "--title",
            "Start fresh",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"]["number"], 1);
}

#[test]
fn issue_create_rolls_back_the_issue_and_domain_event_when_success_audit_cannot_be_written() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_issue_create_success_audits
             BEFORE INSERT ON audit_events
             WHEN NEW.operation = 'issue_create' AND NEW.success = 1
             BEGIN SELECT RAISE(ABORT, 'success audit unavailable'); END;",
        )
        .unwrap();
    drop(connection);

    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Build local core",
        ])
        .assert()
        .code(10);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM issues", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM domain_events WHERE event_type = 'issue_created'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn issue_show_returns_the_created_issue_in_the_json_envelope() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
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
            "Build local core",
        ])
        .assert()
        .success();

    let output = app
        .command()
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    assert_eq!(response["data"]["number"], 1);
    assert_eq!(response["data"]["title"], "Build local core");
}

#[test]
fn issue_commands_distinguish_missing_and_unknown_projects_and_issues() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();

    let missing_project = app
        .command()
        .args(["issue", "create", "--title", "Build local core", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing_project.status.code(), Some(2));
    let missing_project_error: serde_json::Value =
        serde_json::from_slice(&missing_project.stderr).unwrap();
    assert_eq!(missing_project_error["schema_version"], 1);
    assert_eq!(missing_project_error["error"]["code"], "invalid_input");

    let unknown_project = app
        .command()
        .args([
            "issue",
            "create",
            "--project",
            "missing",
            "--title",
            "Build local core",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(unknown_project.status.code(), Some(3));
    let unknown_project_error: serde_json::Value =
        serde_json::from_slice(&unknown_project.stderr).unwrap();
    assert_eq!(unknown_project_error["schema_version"], 1);
    assert_eq!(unknown_project_error["error"]["code"], "not_found");

    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    app.command()
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("not_found"));
}

#[test]
fn issue_create_validates_titles() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    app.command()
        .args(["issue", "create", "--project", "bettr", "--title", "  "])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));
    let long_title = "a".repeat(501);
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            &long_title,
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));
}

#[test]
fn issue_create_parse_errors_in_json_mode_use_the_error_envelope() {
    let app = crate::support::TestApp::new();

    for arguments in [
        vec!["issue", "create", "--project", "bettr", "--json"],
        vec![
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Build local core",
            "--priority",
            "highest",
            "--json",
        ],
    ] {
        let output = app.command().args(arguments).output().unwrap();

        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["schema_version"], 1);
        assert_eq!(response["error"]["code"], "invalid_input");
        assert!(!response["error"]["message"].as_str().unwrap().is_empty());
    }
}

#[test]
fn issue_show_preserves_terminal_escape_sequences_as_text() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
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
            "\u{1b}[31mnot red",
            "--body",
            "\u{1b}[0mstill text",
        ])
        .assert()
        .success();

    let output = app
        .command()
        .args(["issue", "show", "1", "--project", "bettr"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let human_output = String::from_utf8(output.stdout).unwrap();
    assert!(human_output.contains("bettr#1"));
    assert!(human_output.contains("\\u{1b}[31mnot red"));
    assert!(human_output.contains("\\u{1b}[0mstill text"));
}

#[test]
fn issue_show_escapes_terminal_controls_in_the_project_name() {
    let app = crate::support::TestApp::new();
    let project = "\u{1b}[31mbettr";
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", project])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            project,
            "--title",
            "Build local core",
        ])
        .assert()
        .success();

    let output = app
        .command()
        .args(["issue", "show", "1", "--project", project])
        .output()
        .unwrap();

    assert!(output.status.success());
    let human_output = String::from_utf8(output.stdout).unwrap();
    assert!(human_output.contains("\\u{1b}[31mbettr#1"));
}
