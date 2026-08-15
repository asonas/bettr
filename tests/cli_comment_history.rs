mod support;

fn initialized_issue() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .env("BETTR_OPERATOR", "project-owner")
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    app.command()
        .env("BETTR_OPERATOR", "issue-author")
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
    app
}

fn issue_json(app: &crate::support::TestApp) -> serde_json::Value {
    let output = app
        .command()
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

#[test]
fn comments_have_immutable_identity_utc_timestamps_context_and_insertion_order() {
    let app = initialized_issue();
    let before = issue_json(&app);

    let first = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-7")
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "Implemented the patch",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(first.status.success());
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    let first = &first["data"];
    assert_eq!(first["body"], "Implemented the patch");
    assert_eq!(first["context"]["kind"], "agent");
    assert_eq!(first["context"]["agent"], "codex");
    assert_eq!(first["context"]["session_id"], "session-7");
    assert!(first["context"]["operator"].is_null());

    let second = app
        .command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "Verified the result",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(second.status.success());
    let second: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    let second = &second["data"];
    assert_eq!(second["context"]["kind"], "human");
    assert_eq!(second["context"]["operator"], "reviewer");

    let first_id = first["id"].as_str().unwrap();
    let second_id = second["id"].as_str().unwrap();
    assert_ne!(first_id, second_id);
    uuid::Uuid::parse_str(first_id).unwrap();
    uuid::Uuid::parse_str(second_id).unwrap();
    for comment in [first, second] {
        let created_at = comment["created_at"].as_str().unwrap();
        assert!(created_at.ends_with('Z'));
        created_at.parse::<chrono::DateTime<chrono::Utc>>().unwrap();
    }

    let after = issue_json(&app);
    assert_eq!(after["revision"], before["revision"]);
    assert_ne!(after["updated_at"], before["updated_at"]);

    let history = app
        .command()
        .args(["issue", "history", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    assert!(history.status.success());
    let history: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    let events = history["data"].as_array().unwrap();
    let comment_events = events
        .iter()
        .filter(|event| event["event_type"] == "comment_added")
        .collect::<Vec<_>>();
    assert_eq!(comment_events.len(), 2);
    assert_eq!(comment_events[0]["metadata"]["comment_id"], first_id);
    assert_eq!(
        comment_events[0]["metadata"]["body"],
        "Implemented the patch"
    );
    assert_eq!(comment_events[1]["metadata"]["comment_id"], second_id);
    assert_eq!(comment_events[1]["metadata"]["body"], "Verified the result");

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let stored = connection
        .prepare("SELECT id, body FROM comments ORDER BY rowid ASC")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        stored,
        vec![
            (first_id.to_owned(), "Implemented the patch".to_owned()),
            (second_id.to_owned(), "Verified the result".to_owned()),
        ]
    );
}

#[test]
fn comment_cli_has_no_edit_or_delete_operation() {
    let app = initialized_issue();

    for operation in ["edit", "delete"] {
        let output = app
            .command()
            .args([
                "issue",
                "comment",
                operation,
                "--project",
                "bettr",
                "--body",
                "not allowed",
                "--json",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
    }

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM comments", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn issue_history_projects_domain_events_in_sequence_without_audit_reads() {
    let app = initialized_issue();
    app.command()
        .env("BETTR_AGENT", "editor")
        .env("BETTR_SESSION_ID", "edit-session")
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--priority",
            "urgent",
        ])
        .assert()
        .success();
    app.command()
        .env("BETTR_OPERATOR", "commenter")
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "Ready to start",
        ])
        .assert()
        .success();
    app.command()
        .env("BETTR_OPERATOR", "starter")
        .args([
            "issue",
            "start",
            "1",
            "--project",
            "bettr",
            "--revision",
            "2",
        ])
        .assert()
        .success();
    app.command()
        .args(["issue", "show", "1", "--project", "bettr"])
        .assert()
        .success();
    app.command()
        .args(["issue", "list", "--project", "bettr"])
        .assert()
        .success();

    let output = app
        .command()
        .env("BETTR_OPERATOR", "auditor")
        .args(["issue", "history", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let events = response["data"].as_array().unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event["event_type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "issue_created",
            "issue_updated",
            "comment_added",
            "issue_started",
        ]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["revision"].as_i64())
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2), None, Some(3)]
    );
    assert!(events.windows(2).all(|pair| {
        pair[0]["sequence"].as_i64().unwrap() < pair[1]["sequence"].as_i64().unwrap()
    }));
    for event in events {
        let occurred_at = event["created_at"].as_str().unwrap();
        assert!(occurred_at.ends_with('Z'));
        occurred_at
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        assert!(event["context"]["kind"].is_string());
        assert!(event["metadata"].is_object());
        assert!(event.get("operation").is_none());
        assert!(event.get("success").is_none());
    }
    assert_eq!(events[0]["context"]["operator"], "issue-author");
    assert_eq!(events[1]["context"]["agent"], "editor");
    assert_eq!(events[1]["context"]["session_id"], "edit-session");
    assert_eq!(events[2]["context"]["operator"], "commenter");
    assert_eq!(events[3]["context"]["operator"], "starter");
    assert_eq!(events[1]["metadata"]["changes"]["priority"], "urgent");
    assert_eq!(events[2]["metadata"]["body"], "Ready to start");

    let serialized = serde_json::to_string(events).unwrap();
    assert!(!serialized.contains("issue_show"));
    assert!(!serialized.contains("issue_list"));
    assert!(!serialized.contains("issue_history"));
}

#[test]
fn comment_and_activity_update_roll_back_when_the_domain_event_fails() {
    let app = initialized_issue();
    let before = issue_json(&app);
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_comment_domain_events
             BEFORE INSERT ON domain_events
             WHEN NEW.event_type = 'comment_added'
             BEGIN SELECT RAISE(ABORT, 'comment event unavailable'); END;",
        )
        .unwrap();
    drop(connection);

    app.command()
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "Must be atomic",
        ])
        .assert()
        .code(10);

    let after = issue_json(&app);
    assert_eq!(after["revision"], before["revision"]);
    assert_eq!(after["updated_at"], before["updated_at"]);
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM comments", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
