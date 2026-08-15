mod support;

fn json_data(output: std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

fn initialized_issue() -> (
    crate::support::TestApp,
    serde_json::Value,
    serde_json::Value,
) {
    let app = crate::support::TestApp::new();
    app.command()
        .env("BETTR_OPERATOR", "initializer")
        .arg("init")
        .assert()
        .success();
    let project = json_data(
        app.command()
            .env("BETTR_OPERATOR", "project-owner")
            .args(["project", "create", "bettr", "--json"])
            .output()
            .unwrap(),
    );
    let issue = json_data(
        app.command()
            .env("BETTR_AGENT", "codex")
            .env("BETTR_SESSION_ID", "session-8")
            .env("UNRELATED_SECRET", "must-not-appear")
            .args([
                "issue",
                "create",
                "--project",
                "bettr",
                "--title",
                "secret title",
                "--body",
                "secret body",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    (app, project, issue)
}

#[test]
fn audit_list_exposes_complete_safe_success_and_failure_events() {
    let (app, project, issue) = initialized_issue();

    app.command()
        .env("BETTR_AGENT", "reader")
        .env("BETTR_SESSION_ID", "read-session")
        .args(["issue", "show", "1", "--project", "bettr"])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "start",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
        ])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "start",
            "1",
            "--project",
            "bettr",
            "--revision",
            "2",
            "--json",
        ])
        .assert()
        .code(4);
    app.command()
        .args(["issue", "show", "404", "--project", "bettr", "--json"])
        .assert()
        .code(3);
    app.command()
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--title",
            "another secret title",
            "--json",
        ])
        .assert()
        .code(4);
    app.command()
        .args([
            "issue",
            "comment",
            "0",
            "--project",
            "bettr",
            "--body",
            "secret comment",
            "--json",
        ])
        .assert()
        .code(2);

    let events = json_data(
        app.command()
            .args(["audit", "list", "--json"])
            .output()
            .unwrap(),
    );
    let events = events.as_array().unwrap();
    assert!(events.len() >= 9);
    for event in events {
        uuid::Uuid::parse_str(event["id"].as_str().unwrap()).unwrap();
        assert!(event["operation"].is_string());
        assert!(event["started_at"].as_str().unwrap().ends_with('Z'));
        assert!(event["finished_at"].as_str().unwrap().ends_with('Z'));
        assert!(event["outcome"] == "success" || event["outcome"] == "failure");
        assert!(event["exit_code"].is_number());
        assert!(event["context"]["kind"].is_string());
    }

    let create = events
        .iter()
        .find(|event| event["operation"] == "issue_create")
        .unwrap();
    assert_eq!(create["project"]["id"], project["id"]);
    assert_eq!(create["project"]["name"], "bettr");
    assert_eq!(create["target"]["kind"], "issue");
    assert_eq!(create["target"]["id"], issue["id"]);
    assert_eq!(create["context"]["kind"], "agent");
    assert_eq!(create["context"]["agent"], "codex");
    assert_eq!(create["context"]["session_id"], "session-8");
    assert_eq!(create["revision"], 1);
    assert_eq!(create["outcome"], "success");
    assert_eq!(create["exit_code"], 0);

    let invalid_transition = events
        .iter()
        .find(|event| event["operation"] == "issue_start" && event["outcome"] == "failure")
        .unwrap();
    assert_eq!(invalid_transition["project"]["id"], project["id"]);
    assert_eq!(invalid_transition["target"]["id"], issue["id"]);
    assert_eq!(invalid_transition["revision"], 2);
    assert_eq!(invalid_transition["exit_code"], 4);

    let missing = events
        .iter()
        .find(|event| event["operation"] == "issue_show" && event["outcome"] == "failure")
        .unwrap();
    assert_eq!(missing["project"]["id"], project["id"]);
    assert!(missing["target"].is_null());
    assert_eq!(missing["exit_code"], 3);

    let conflict = events
        .iter()
        .find(|event| event["operation"] == "issue_edit" && event["outcome"] == "failure")
        .unwrap();
    assert_eq!(conflict["target"]["id"], issue["id"]);
    assert_eq!(conflict["revision"], 2);
    assert_eq!(conflict["exit_code"], 4);

    let malformed = events
        .iter()
        .find(|event| event["operation"] == "issue_comment" && event["outcome"] == "failure")
        .unwrap();
    assert_eq!(malformed["project"]["id"], project["id"]);
    assert_eq!(malformed["exit_code"], 2);

    let serialized = serde_json::to_string(events).unwrap();
    for forbidden in [
        "secret title",
        "secret body",
        "another secret title",
        "secret comment",
        "must-not-appear",
        "UNRELATED_SECRET",
        "raw_argv",
    ] {
        assert!(!serialized.contains(forbidden), "audit leaked {forbidden}");
    }
}

#[test]
fn transition_constructor_failures_are_audited() {
    let (app, _project, _issue) = initialized_issue();
    let cases = [
        (
            "block",
            &["--reason", " ", "--wait-kind", "dependency"][..],
            "issue_block",
        ),
        (
            "complete",
            &["--summary", " ", "--verification", "cargo test passed"][..],
            "issue_complete",
        ),
        ("cancel", &["--reason", " "][..], "issue_cancel"),
        ("reopen", &["--reason", " "][..], "issue_reopen"),
    ];

    for (command, arguments, _operation) in cases {
        let mut command_arguments = vec!["issue", command, "1", "--project", "bettr"];
        command_arguments.extend_from_slice(arguments);
        command_arguments.extend_from_slice(&["--revision", "1", "--json"]);
        app.command().args(command_arguments).assert().code(2);
    }

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    for (_, _, operation) in cases {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM audit_events
                     WHERE operation = ?1 AND success = 0 AND exit_code = 2",
                    [operation],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "missing failure audit for {operation}"
        );
    }
}

#[test]
fn audit_list_filters_safe_events_by_every_supported_dimension() {
    let (app, project, _issue) = initialized_issue();
    let project_id = project["id"].as_str().unwrap();
    let before = chrono::Utc::now().to_rfc3339();
    app.command()
        .env("BETTR_AGENT", "reader")
        .env("BETTR_SESSION_ID", "filter-session")
        .args(["issue", "show", "1", "--project", "bettr"])
        .assert()
        .success();
    let after = chrono::Utc::now().to_rfc3339();

    let filtered = |arguments: &[&str]| {
        let mut command = app.command();
        command.args(["audit", "list", "--json"]);
        command.args(arguments);
        let events = json_data(command.output().unwrap());
        assert!(!events.as_array().unwrap().is_empty());
        events
    };
    let by_project = filtered(&["--project-id", project_id]);
    assert!(
        by_project
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["project"]["id"] == project_id)
    );
    let by_operation = filtered(&["--operation", "issue_show"]);
    assert!(
        by_operation
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["operation"] == "issue_show")
    );
    let by_outcome = filtered(&["--outcome", "success"]);
    assert!(
        by_outcome
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["outcome"] == "success")
    );
    let by_kind = filtered(&["--kind", "agent"]);
    assert!(
        by_kind
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["context"]["kind"] == "agent")
    );
    let by_agent = filtered(&["--agent", "reader"]);
    assert!(
        by_agent
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["context"]["agent"] == "reader")
    );
    let by_session = filtered(&["--session-id", "filter-session"]);
    assert!(
        by_session
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["context"]["session_id"] == "filter-session")
    );
    let by_time = filtered(&["--after", &before, "--before", &after]);
    let before_timestamp = before.parse::<chrono::DateTime<chrono::Utc>>().unwrap();
    let after_timestamp = after.parse::<chrono::DateTime<chrono::Utc>>().unwrap();
    assert!(by_time.as_array().unwrap().iter().all(|event| {
        let started = event["started_at"]
            .as_str()
            .unwrap()
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        let finished = event["finished_at"]
            .as_str()
            .unwrap()
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap();
        started <= after_timestamp && finished >= before_timestamp
    }));

    let exact = json_data(
        app.command()
            .args([
                "audit",
                "list",
                "--operation",
                "issue_show",
                "--outcome",
                "success",
                "--kind",
                "agent",
                "--agent",
                "reader",
                "--session-id",
                "filter-session",
                "--project-id",
                project_id,
                "--after",
                &before,
                "--before",
                &after,
                "--json",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(exact.as_array().unwrap().len(), 1);
    assert_eq!(exact[0]["operation"], "issue_show");
}

#[test]
fn audit_human_output_is_concise_and_escaped() {
    let (app, _project, _issue) = initialized_issue();
    let output = app
        .command()
        .args(["audit", "list", "--operation", "issue_create"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("issue_create"));
    assert!(stdout.contains("success"));
    assert!(!stdout.contains("secret title"));
    assert!(!stdout.contains("secret body"));
}

#[test]
fn malformed_audit_filter_that_reaches_the_app_is_audited() {
    let (app, _project, _issue) = initialized_issue();

    app.command()
        .args(["audit", "list", "--outcome", "unknown", "--json"])
        .assert()
        .code(2);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let (success, exit_code): (i64, i64) = connection
        .query_row(
            "SELECT success, exit_code FROM audit_events
             WHERE operation = 'audit_list' ORDER BY rowid DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(success, 0);
    assert_eq!(exit_code, 2);
}
