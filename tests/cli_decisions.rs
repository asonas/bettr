mod support;

fn initialized_app() -> crate::support::TestApp {
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
            "Needs a decision",
        ])
        .assert()
        .success();
    app
}

fn json_data(output: &std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

#[test]
fn decision_requests_block_issue_and_resolve_with_human_context() {
    let app = initialized_app();

    let first = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The current parser cannot handle this input.",
            "--question",
            "Which parser should we use?",
            "--option",
            "Use the streaming parser.",
            "--option",
            "Keep the current parser.",
            "--recommendation",
            "Use the streaming parser.",
            "--resume-condition",
            "The selected parser is implemented and verified.",
            "--background",
            "The current parser cannot handle this input.",
            "--json",
        ])
        .output()
        .unwrap();
    let first = json_data(&first);
    assert_eq!(first["issue"], "bettr#1");
    assert_eq!(first["status"], "open");
    assert_eq!(
        first["blocker"],
        "The current parser cannot handle this input."
    );
    assert_eq!(first["question"], "Which parser should we use?");
    assert_eq!(first["options"][0], "Use the streaming parser.");
    assert_eq!(first["options"][1], "Keep the current parser.");
    assert_eq!(first["recommendation"], "Use the streaming parser.");
    assert_eq!(
        first["resume_condition"],
        "The selected parser is implemented and verified."
    );
    assert_eq!(
        first["background"],
        "The current parser cannot handle this input."
    );
    assert_eq!(first["requester_kind"], "agent");
    assert_eq!(first["requester_name"], "codex");
    assert_eq!(first["requester_session_id"], "session-a");
    let first_id = first["id"].as_str().unwrap().to_owned();

    let second = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-b")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The migration has two possible defaults.",
            "--question",
            "Should the compatibility mode stay enabled?",
            "--option",
            "Keep compatibility mode enabled.",
            "--option",
            "Disable compatibility mode.",
            "--recommendation",
            "Disable compatibility mode.",
            "--resume-condition",
            "The selected default is recorded in the migration.",
            "--background",
            "The migration has two possible defaults.",
            "--json",
        ])
        .output()
        .unwrap();
    let second = json_data(&second);
    let second_id = second["id"].as_str().unwrap().to_owned();
    assert_ne!(first_id, second_id);

    let status = app.command().args(["status", "--json"]).output().unwrap();
    let status = json_data(&status);
    assert_eq!(status["attention"].as_array().unwrap().len(), 1);
    assert_eq!(status["attention"][0]["number"], 1);
    assert_eq!(status["attention"][0]["state"], "blocked");
    assert!(status["blocked"].as_array().unwrap().is_empty());

    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "resolve",
            &first_id,
            "--answer",
            "Use the streaming parser.",
            "--next-state",
            "done",
            "--json",
        ])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));

    let resolved = app
        .command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            &first_id,
            "--answer",
            "Use the streaming parser.",
            "--next-state",
            "blocked",
            "--reason",
            "Waiting for the rollout window.",
            "--wait-kind",
            "human",
            "--json",
        ])
        .output()
        .unwrap();
    let resolved = json_data(&resolved);
    assert_eq!(resolved["id"], first_id);
    assert_eq!(resolved["status"], "resolved");
    assert_eq!(resolved["answer"], "Use the streaming parser.");
    assert_eq!(resolved["resolver_kind"], "human");
    assert_eq!(resolved["resolver_name"], "reviewer");
    assert_eq!(resolved["resolver_session_id"], serde_json::Value::Null);

    let still_waiting = app.command().args(["status", "--json"]).output().unwrap();
    let still_waiting = json_data(&still_waiting);
    assert_eq!(still_waiting["attention"].as_array().unwrap().len(), 1);

    let resolved_second = app
        .command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            &second_id,
            "--answer",
            "Keep compatibility mode disabled.",
            "--next-state",
            "todo",
            "--json",
        ])
        .output()
        .unwrap();
    let resolved_second = json_data(&resolved_second);
    assert_eq!(resolved_second["status"], "resolved");
    assert_eq!(resolved_second["resolver_name"], "reviewer");

    let status = app.command().args(["status", "--json"]).output().unwrap();
    let status = json_data(&status);
    assert!(status["attention"].as_array().unwrap().is_empty());
    assert_eq!(status["active"][0]["state"], "todo");
}

#[test]
fn decision_resolution_releases_the_requester_lease_for_reclaim() {
    let app = initialized_app();

    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "claim", "1", "--project", "bettr", "--json"])
        .assert()
        .success();

    let request = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The current parser cannot handle this input.",
            "--question",
            "Which parser should we use?",
            "--option",
            "Use the streaming parser.",
            "--option",
            "Keep the current parser.",
            "--recommendation",
            "Use the streaming parser.",
            "--resume-condition",
            "The selected parser is implemented and verified.",
            "--background",
            "The current parser cannot handle this input.",
            "--json",
        ])
        .output()
        .unwrap();
    let request = json_data(&request);
    let request_id = request["id"].as_str().unwrap();

    app.command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            request_id,
            "--answer",
            "Use the streaming parser.",
            "--next-state",
            "todo",
            "--json",
        ])
        .assert()
        .success();

    let reclaimed = app
        .command()
        .env("BETTR_AGENT", "worker")
        .env("BETTR_SESSION_ID", "session-b")
        .args(["issue", "claim", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    let reclaimed = json_data(&reclaimed);
    assert_eq!(reclaimed["issue"]["state"], "in_progress");
    assert_eq!(reclaimed["lease"]["session_id"], "session-b");
}

#[test]
fn decision_resolution_requires_human_context() {
    let app = initialized_app();

    let request = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The current parser cannot handle this input.",
            "--question",
            "Which parser should we use?",
            "--option",
            "Use the streaming parser.",
            "--option",
            "Keep the current parser.",
            "--recommendation",
            "Use the streaming parser.",
            "--resume-condition",
            "The selected parser is implemented and verified.",
            "--background",
            "The current parser cannot handle this input.",
            "--json",
        ])
        .output()
        .unwrap();
    let request = json_data(&request);
    let request_id = request["id"].as_str().unwrap();

    app.command()
        .env("BETTR_AGENT", "another-agent")
        .env("BETTR_SESSION_ID", "session-b")
        .args([
            "decision",
            "resolve",
            request_id,
            "--answer",
            "Use the streaming parser.",
            "--next-state",
            "todo",
            "--json",
        ])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("human"));
}

#[test]
fn decision_resolution_rejects_in_progress_without_a_lease() {
    let app = initialized_app();

    let request = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The current parser cannot handle this input.",
            "--question",
            "Which parser should we use?",
            "--option",
            "Use the streaming parser.",
            "--option",
            "Keep the current parser.",
            "--recommendation",
            "Use the streaming parser.",
            "--resume-condition",
            "The selected parser is implemented and verified.",
            "--background",
            "The current parser cannot handle this input.",
            "--json",
        ])
        .output()
        .unwrap();
    let request = json_data(&request);
    let request_id = request["id"].as_str().unwrap();

    app.command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            request_id,
            "--answer",
            "Use the streaming parser.",
            "--next-state",
            "in_progress",
            "--json",
        ])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("lease"));
}

#[test]
fn decision_resolution_requires_state_specific_metadata_and_emits_transition_event() {
    let app = initialized_app();

    let request = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The current parser cannot handle this input.",
            "--question",
            "Which parser should we use?",
            "--option",
            "Use the streaming parser.",
            "--option",
            "Keep the current parser.",
            "--recommendation",
            "Use the streaming parser.",
            "--resume-condition",
            "The selected parser is implemented and verified.",
            "--background",
            "The current parser cannot handle this input.",
            "--json",
        ])
        .output()
        .unwrap();
    let request = json_data(&request);
    let request_id = request["id"].as_str().unwrap();

    app.command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            request_id,
            "--answer",
            "Use the streaming parser.",
            "--next-state",
            "done",
            "--json",
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("summary"));

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT state FROM issues WHERE number = 1", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "blocked"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM decision_requests WHERE id = ?1",
                [request_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "open"
    );
    drop(connection);

    app.command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            request_id,
            "--answer",
            "Use the streaming parser.",
            "--next-state",
            "done",
            "--summary",
            "Selected the streaming parser.",
            "--verification",
            "Reviewed the parser integration tests.",
            "--json",
        ])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let (event_type, metadata): (String, String) = connection
        .query_row(
            "SELECT event_type, metadata_json FROM domain_events ORDER BY sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(event_type, "issue_completed");
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["request_id"], request_id);
    assert_eq!(metadata["summary"], "Selected the streaming parser.");
    assert_eq!(
        metadata["verification"],
        "Reviewed the parser integration tests."
    );
}

#[test]
fn decision_resolution_requires_block_and_cancel_metadata() {
    let app = initialized_app();

    let request = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The current parser cannot handle this input.",
            "--question",
            "Which parser should we use?",
            "--option",
            "Use the streaming parser.",
            "--option",
            "Keep the current parser.",
            "--recommendation",
            "Use the streaming parser.",
            "--resume-condition",
            "The selected parser is implemented and verified.",
            "--background",
            "The current parser cannot handle this input.",
            "--json",
        ])
        .output()
        .unwrap();
    let request = json_data(&request);
    let request_id = request["id"].as_str().unwrap();

    app.command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            request_id,
            "--answer",
            "Keep the current parser.",
            "--next-state",
            "cancelled",
            "--json",
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("reason"));

    app.command()
        .env("BETTR_OPERATOR", "reviewer")
        .args([
            "decision",
            "resolve",
            request_id,
            "--answer",
            "Keep the current parser.",
            "--next-state",
            "cancelled",
            "--reason",
            "The parser change is no longer needed.",
            "--json",
        ])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let (event_type, metadata): (String, String) = connection
        .query_row(
            "SELECT event_type, metadata_json FROM domain_events ORDER BY sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(event_type, "issue_cancelled");
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["reason"], "The parser change is no longer needed.");
}

#[test]
fn decision_requests_validate_input_and_keep_unresolved_requests_from_done() {
    let app = initialized_app();

    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The current parser cannot handle this input.",
            "--question",
            " ",
            "--option",
            "Use the streaming parser.",
            "--option",
            "Keep the current parser.",
            "--recommendation",
            "Use the streaming parser.",
            "--resume-condition",
            "The selected parser is implemented and verified.",
            "--background",
            "context",
            "--json",
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));

    let request = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args([
            "decision",
            "request",
            "1",
            "--project",
            "bettr",
            "--blocker",
            "The target changes the rollout plan.",
            "--question",
            "Choose a deployment target",
            "--option",
            "Deploy to staging.",
            "--option",
            "Deploy to production.",
            "--recommendation",
            "Deploy to staging.",
            "--resume-condition",
            "The selected target is approved and verified.",
            "--background",
            "The target changes the rollout plan.",
            "--json",
        ])
        .output()
        .unwrap();
    let request = json_data(&request);
    let request_id = request["id"].as_str().unwrap();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute(
            "UPDATE issues SET state = 'in_progress' WHERE number = 1",
            [],
        )
        .unwrap();
    drop(connection);

    app.command()
        .args([
            "issue",
            "complete",
            "1",
            "--project",
            "bettr",
            "--revision",
            "2",
            "--summary",
            "finished",
            "--verification",
            "passed",
            "--json",
        ])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM decision_requests WHERE id = ?1",
                [request_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "open"
    );
    assert_eq!(
        connection
            .query_row("SELECT state FROM issues WHERE number = 1", [], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap(),
        "in_progress"
    );
}
