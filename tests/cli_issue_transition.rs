mod support;

struct AllowedTransition {
    from: &'static str,
    command: &'static str,
    arguments: &'static [&'static str],
    to: &'static str,
    event_type: &'static str,
}

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
            "Build local core",
        ])
        .assert()
        .success();
    app
}

fn set_issue_state(app: &crate::support::TestApp, state: &str, revision: i64) {
    rusqlite::Connection::open(&app.database)
        .unwrap()
        .execute(
            "UPDATE issues SET state = ?1, revision = ?2 WHERE number = 1",
            rusqlite::params![state, revision],
        )
        .unwrap();
}

fn issue_snapshot(app: &crate::support::TestApp) -> (String, i64, String, Option<String>) {
    rusqlite::Connection::open(&app.database)
        .unwrap()
        .query_row(
            "SELECT state, revision, title, body FROM issues WHERE number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
}

fn domain_event_count(app: &crate::support::TestApp) -> i64 {
    rusqlite::Connection::open(&app.database)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM domain_events", [], |row| row.get(0))
        .unwrap()
}

fn transition_output(
    app: &crate::support::TestApp,
    command: &str,
    extra_arguments: &[&str],
) -> std::process::Output {
    let mut process = app.command();
    process.args([
        "issue",
        command,
        "1",
        "--project",
        "bettr",
        "--revision",
        "7",
        "--json",
    ]);
    process.args(extra_arguments).output().unwrap()
}

#[test]
fn every_allowed_transition_changes_state_increments_revision_and_appends_one_event() {
    let cases = [
        AllowedTransition {
            from: "todo",
            command: "start",
            arguments: &[],
            to: "in_progress",
            event_type: "issue_started",
        },
        AllowedTransition {
            from: "in_progress",
            command: "block",
            arguments: &["--reason", "Waiting for review", "--wait-kind", "human"],
            to: "blocked",
            event_type: "issue_blocked",
        },
        AllowedTransition {
            from: "blocked",
            command: "resume",
            arguments: &[],
            to: "in_progress",
            event_type: "issue_resumed",
        },
        AllowedTransition {
            from: "in_progress",
            command: "complete",
            arguments: &[
                "--summary",
                "Implemented transitions",
                "--verification",
                "cargo test passed",
            ],
            to: "done",
            event_type: "issue_completed",
        },
        AllowedTransition {
            from: "in_progress",
            command: "cancel",
            arguments: &["--reason", "No longer needed"],
            to: "cancelled",
            event_type: "issue_cancelled",
        },
        AllowedTransition {
            from: "blocked",
            command: "cancel",
            arguments: &["--reason", "Dependency removed"],
            to: "cancelled",
            event_type: "issue_cancelled",
        },
        AllowedTransition {
            from: "done",
            command: "reopen",
            arguments: &["--reason", "Verification failed"],
            to: "todo",
            event_type: "issue_reopened",
        },
        AllowedTransition {
            from: "cancelled",
            command: "reopen",
            arguments: &["--reason", "Work is needed again"],
            to: "todo",
            event_type: "issue_reopened",
        },
    ];

    for case in cases {
        let app = initialized_app();
        set_issue_state(&app, case.from, 7);
        let events_before = domain_event_count(&app);

        let output = transition_output(&app, case.command, case.arguments);

        assert!(
            output.status.success(),
            "{} -> {} failed: {}",
            case.from,
            case.to,
            String::from_utf8_lossy(&output.stderr)
        );
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["data"]["state"], case.to);
        assert_eq!(response["data"]["revision"], 8);
        assert_eq!(domain_event_count(&app), events_before + 1);

        let connection = rusqlite::Connection::open(&app.database).unwrap();
        let (event_type, metadata): (String, String) = connection
            .query_row(
                "SELECT event_type, metadata_json FROM domain_events ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(event_type, case.event_type);
        let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
        assert_eq!(metadata["from_state"], case.from);
        assert_eq!(metadata["to_state"], case.to);
        assert_eq!(metadata["revision"], 8);
    }
}

#[test]
fn representative_disallowed_transitions_leave_the_issue_unchanged() {
    let cases = [
        (
            "todo",
            "block",
            &["--reason", "wait", "--wait-kind", "human"][..],
        ),
        (
            "todo",
            "complete",
            &["--summary", "not started", "--verification", "not run"][..],
        ),
        ("blocked", "start", &[][..]),
        ("done", "cancel", &["--reason", "too late"][..]),
        ("cancelled", "resume", &[][..]),
        ("in_progress", "reopen", &["--reason", "already active"][..]),
    ];

    for (state, command, arguments) in cases {
        let app = initialized_app();
        set_issue_state(&app, state, 7);
        let before = issue_snapshot(&app);
        let events_before = domain_event_count(&app);

        let output = transition_output(&app, command, arguments);

        assert_eq!(output.status.code(), Some(4));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_transition");
        assert_eq!(issue_snapshot(&app), before);
        assert_eq!(domain_event_count(&app), events_before);
    }
}

#[test]
fn transition_commands_require_revision_and_transition_metadata() {
    let app = crate::support::TestApp::new();
    let missing_argument_cases = [
        vec!["issue", "start", "1", "--project", "bettr"],
        vec![
            "issue",
            "block",
            "1",
            "--project",
            "bettr",
            "--reason",
            "wait",
            "--wait-kind",
            "human",
        ],
        vec![
            "issue",
            "block",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--wait-kind",
            "human",
        ],
        vec![
            "issue",
            "block",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--reason",
            "wait",
        ],
        vec!["issue", "resume", "1", "--project", "bettr"],
        vec![
            "issue",
            "complete",
            "1",
            "--project",
            "bettr",
            "--summary",
            "done",
            "--verification",
            "passed",
        ],
        vec![
            "issue",
            "complete",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--verification",
            "passed",
        ],
        vec![
            "issue",
            "complete",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--summary",
            "done",
        ],
        vec![
            "issue",
            "cancel",
            "1",
            "--project",
            "bettr",
            "--reason",
            "not needed",
        ],
        vec![
            "issue",
            "cancel",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
        ],
        vec![
            "issue",
            "reopen",
            "1",
            "--project",
            "bettr",
            "--reason",
            "needed again",
        ],
        vec![
            "issue",
            "reopen",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
        ],
    ];

    for arguments in missing_argument_cases {
        let mut arguments = arguments;
        arguments.push("--json");
        let output = app.command().args(arguments).output().unwrap();

        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
    }
}

#[test]
fn transition_constructors_reject_blank_required_metadata() {
    let cases = [
        ("block", &["--reason", " ", "--wait-kind", "dependency"][..]),
        (
            "complete",
            &["--summary", " ", "--verification", "cargo test passed"][..],
        ),
        (
            "complete",
            &["--summary", "done", "--verification", " "][..],
        ),
        ("cancel", &["--reason", " "][..]),
        ("reopen", &["--reason", " "][..]),
    ];

    for (command, arguments) in cases {
        let app = initialized_app();
        set_issue_state(&app, "in_progress", 7);
        let output = transition_output(&app, command, arguments);

        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
        assert_eq!(issue_snapshot(&app).1, 7);
    }
}

#[test]
fn stale_revision_reports_the_current_revision_without_changing_issue_data() {
    let app = initialized_app();
    let first = app
        .command()
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
        .output()
        .unwrap();
    assert!(first.status.success());
    let after_first = issue_snapshot(&app);

    let stale = app
        .command()
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
        .output()
        .unwrap();

    assert_eq!(stale.status.code(), Some(4));
    let response: serde_json::Value = serde_json::from_slice(&stale.stderr).unwrap();
    assert_eq!(response["error"]["code"], "revision_conflict");
    assert_eq!(response["error"]["details"]["current_revision"], 2);
    assert_eq!(issue_snapshot(&app), after_first);
}

#[test]
fn concurrent_processes_using_the_same_revision_allow_exactly_one_update() {
    let app = initialized_app();
    let binary = assert_cmd::cargo::cargo_bin!("bettr");
    let mut first = std::process::Command::new(binary);
    first
        .arg("--database")
        .arg(&app.database)
        .env_clear()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    first.args([
        "issue",
        "start",
        "1",
        "--project",
        "bettr",
        "--revision",
        "1",
        "--json",
    ]);
    let mut second = std::process::Command::new(binary);
    second
        .arg("--database")
        .arg(&app.database)
        .env_clear()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    second.args([
        "issue",
        "start",
        "1",
        "--project",
        "bettr",
        "--revision",
        "1",
        "--json",
    ]);

    let first = first.spawn().unwrap();
    let second = second.spawn().unwrap();
    let outputs = [
        first.wait_with_output().unwrap(),
        second.wait_with_output().unwrap(),
    ];

    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1
    );
    let loser = outputs
        .iter()
        .find(|output| !output.status.success())
        .unwrap();
    assert_eq!(loser.status.code(), Some(4));
    let response: serde_json::Value = serde_json::from_slice(&loser.stderr).unwrap();
    assert_eq!(response["error"]["code"], "revision_conflict");
    assert_eq!(response["error"]["details"]["current_revision"], 2);
    assert_eq!(issue_snapshot(&app).0, "in_progress");
    assert_eq!(issue_snapshot(&app).1, 2);
    assert_eq!(domain_event_count(&app), 3);
}
