mod support;

#[test]
fn project_create_returns_the_new_project_in_the_json_envelope() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();

    let output = app
        .command()
        .args(["project", "create", "bettr", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let project = &response["data"];
    let id = project["id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(id.chars().filter(|character| *character == '-').count(), 4);
    assert_eq!(project["name"], "bettr");
    assert_eq!(project["archived"], false);
    assert!(project["created_at"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn duplicate_project_name_returns_project_name_conflict() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    app.command()
        .args(["project", "create", "bettr", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("project_name_conflict"));
}

#[test]
fn project_list_is_ordered_by_name() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "zebra"])
        .assert()
        .success();
    app.command()
        .args(["project", "create", "alpha"])
        .assert()
        .success();

    let output = app
        .command()
        .args(["project", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"][0]["name"], "alpha");
    assert_eq!(response["data"][1]["name"], "zebra");
}

#[test]
fn agent_execution_context_is_written_to_project_audit_without_raw_argv() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();

    let output = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-1")
        .env("BETTR_OPERATOR", "human-operator")
        .args(["project", "create", "bettr", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let project_id = response["data"]["id"].as_str().unwrap();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let audit = connection
        .query_row(
            "SELECT initiator_kind, initiator_name, session_id, operation, success, target_id, metadata_json
             FROM audit_events WHERE operation = 'project_create'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .unwrap();

    assert_eq!(audit.0, "agent");
    assert_eq!(audit.1, "codex");
    assert_eq!(audit.2, "session-1");
    assert_eq!(audit.3, "project_create");
    assert_eq!(audit.4, 1);
    assert_eq!(audit.5, project_id);
    assert!(!audit.6.contains("argv"));
}

#[test]
fn failed_project_creation_is_audited_after_its_transaction_rolls_back() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .code(4);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let failure_metadata: String = connection
        .query_row(
            "SELECT metadata_json FROM audit_events
             WHERE operation = 'project_create' AND success = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(failure_metadata.contains("project_name_conflict"));
}

#[test]
fn project_create_rejects_blank_and_overlong_names() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();

    app.command()
        .args(["project", "create", "   "])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));

    let long_name = "a".repeat(201);
    app.command()
        .args(["project", "create", &long_name])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                 WHERE operation = 'project_create' AND success = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
}
