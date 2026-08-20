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

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let project_id = project["id"].as_str().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT id FROM projects WHERE id = ?1",
                [project_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        project_id
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT project_id FROM domain_events WHERE event_type = 'project_created'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        project_id
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT target_id FROM audit_events
                 WHERE operation = 'project_create' AND success = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        project_id
    );
}

#[test]
fn project_create_rolls_back_project_and_domain_event_when_success_audit_cannot_be_written() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_project_create_success_audits
             BEFORE INSERT ON audit_events
             WHEN NEW.operation = 'project_create' AND NEW.success = 1
             BEGIN SELECT RAISE(ABORT, 'success audit unavailable'); END;",
        )
        .unwrap();
    drop(connection);

    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .code(10);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM domain_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
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
fn project_create_replays_the_same_result_for_an_idempotency_key() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();

    let first = app
        .command()
        .args([
            "project",
            "create",
            "bettr",
            "--idempotency-key",
            "project-create-1",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(first.status.success());

    let second = app
        .command()
        .args([
            "project",
            "create",
            "bettr",
            "--idempotency-key",
            "project-create-1",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(second.status.success());

    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(first, second);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM domain_events WHERE event_type = 'project_created'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE operation = 'project_create' AND success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn project_create_rejects_reusing_an_idempotency_key_for_a_different_payload() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args([
            "project",
            "create",
            "bettr",
            "--idempotency-key",
            "project-create-1",
        ])
        .assert()
        .success();

    app.command()
        .args([
            "project",
            "create",
            "other",
            "--idempotency-key",
            "project-create-1",
            "--json",
        ])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("idempotency_conflict"));

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE operation = 'project_create' AND success = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
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
fn project_human_output_escapes_terminal_controls() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    let project_name = "project\u{1b}[31m";

    let created = app
        .command()
        .args(["project", "create", project_name])
        .output()
        .unwrap();
    assert!(created.status.success());
    let created = String::from_utf8(created.stdout).unwrap();
    assert!(!created.contains('\u{1b}'));
    assert!(created.contains(r"project\u{1b}[31m"));

    let listed = app.command().args(["project", "list"]).output().unwrap();
    assert!(listed.status.success());
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(!listed.contains('\u{1b}'));
    assert!(listed.contains(r"project\u{1b}[31m"));
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
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&audit.6).unwrap(),
        serde_json::json!({})
    );
}

#[test]
fn human_execution_context_falls_back_to_the_os_username() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let context = connection
        .query_row(
            "SELECT initiator_kind, initiator_name, session_id
             FROM audit_events WHERE operation = 'project_create' AND success = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(context.0, "human");
    assert_eq!(context.1.as_deref(), Some(whoami::username().as_str()));
    assert_eq!(context.2, None);
}

#[test]
fn operator_execution_context_is_used_when_no_agent_is_set() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .env("BETTR_OPERATOR", "operator-1")
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let context = connection
        .query_row(
            "SELECT initiator_kind, initiator_name, session_id
             FROM audit_events WHERE operation = 'project_create' AND success = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(context.0, "human");
    assert_eq!(context.1.as_deref(), Some("operator-1"));
    assert_eq!(context.2, None);
}

#[test]
fn agent_execution_context_allows_an_omitted_session() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .env("BETTR_AGENT", "codex")
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let context = connection
        .query_row(
            "SELECT initiator_kind, initiator_name, session_id
             FROM audit_events WHERE operation = 'project_create' AND success = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(context.0, "agent");
    assert_eq!(context.1.as_deref(), Some("codex"));
    assert_eq!(context.2, None);
}

#[test]
fn execution_context_rejects_empty_and_overlong_environment_values() {
    let cases = [
        ("BETTR_AGENT", String::new(), false),
        ("BETTR_AGENT", "a".repeat(201), false),
        ("BETTR_SESSION_ID", String::new(), true),
        ("BETTR_SESSION_ID", "s".repeat(201), true),
        ("BETTR_OPERATOR", String::new(), false),
        ("BETTR_OPERATOR", "o".repeat(201), false),
    ];

    for (variable, value, requires_agent) in cases {
        let app = crate::support::TestApp::new();
        app.command().arg("init").assert().success();
        let mut command = app.command();
        if requires_agent {
            command.env("BETTR_AGENT", "codex");
        }
        command
            .env(variable, value)
            .args(["project", "create", "bettr"])
            .assert()
            .code(2)
            .stderr(predicates::str::contains("invalid_input"));
    }
}

#[test]
fn project_create_surfaces_a_failure_to_persist_its_failure_audit() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_project_create_failure_audits
             BEFORE INSERT ON audit_events
             WHEN NEW.operation = 'project_create' AND NEW.success = 0
             BEGIN SELECT RAISE(ABORT, 'failure audit unavailable'); END;",
        )
        .unwrap();
    drop(connection);

    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .code(10)
        .stderr(predicates::str::contains(
            "failed to persist failure audit for project_create after project_name_conflict",
        ));
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
