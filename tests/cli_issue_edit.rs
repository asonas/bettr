mod support;

fn initialized_issue() -> crate::support::TestApp {
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
            "--body",
            "First vertical slice",
            "--priority",
            "high",
        ])
        .assert()
        .success();
    app
}

fn show_issue(app: &crate::support::TestApp) -> serde_json::Value {
    let output = app
        .command()
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

#[test]
fn issue_edit_changes_fields_preserves_omissions_and_supports_explicit_clearing() {
    let app = initialized_issue();

    let assigned = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-7")
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--title",
            "Build editing and history",
            "--assignee-kind",
            "agent",
            "--assignee-name",
            "reviewer",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        assigned.status.success(),
        "{}",
        String::from_utf8_lossy(&assigned.stderr)
    );
    let assigned: serde_json::Value = serde_json::from_slice(&assigned.stdout).unwrap();
    let issue = &assigned["data"];
    assert_eq!(issue["title"], "Build editing and history");
    assert_eq!(issue["body"], "First vertical slice");
    assert_eq!(issue["priority"], "high");
    assert_eq!(issue["assignee_kind"], "agent");
    assert_eq!(issue["assignee_name"], "reviewer");
    assert_eq!(issue["revision"], 2);

    let changed = app
        .command()
        .env("BETTR_OPERATOR", "Asonas")
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "2",
            "--body",
            "Complete the local workflow",
            "--priority",
            "critical",
            "--assignee-kind",
            "human",
            "--assignee-name",
            "asonas",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(changed.status.success());
    let changed: serde_json::Value = serde_json::from_slice(&changed.stdout).unwrap();
    assert_eq!(changed["data"]["title"], "Build editing and history");
    assert_eq!(changed["data"]["body"], "Complete the local workflow");
    assert_eq!(changed["data"]["priority"], "critical");
    assert_eq!(changed["data"]["assignee_kind"], "human");
    assert_eq!(changed["data"]["assignee_name"], "asonas");
    assert_eq!(changed["data"]["revision"], 3);

    let cleared = app
        .command()
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "3",
            "--clear-body",
            "--clear-priority",
            "--clear-assignee",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(cleared.status.success());
    let cleared: serde_json::Value = serde_json::from_slice(&cleared.stdout).unwrap();
    assert_eq!(cleared["data"]["title"], "Build editing and history");
    assert!(cleared["data"]["body"].is_null());
    assert!(cleared["data"]["priority"].is_null());
    assert!(cleared["data"]["assignee_kind"].is_null());
    assert!(cleared["data"]["assignee_name"].is_null());
    assert_eq!(cleared["data"]["revision"], 4);
}

#[test]
fn issue_edit_replays_after_the_issue_has_moved_to_a_new_revision() {
    let app = initialized_issue();
    let first_arguments = [
        "issue",
        "edit",
        "1",
        "--project",
        "bettr",
        "--revision",
        "1",
        "--title",
        "First title",
        "--idempotency-key",
        "issue-edit-1",
        "--json",
    ];
    let first = app.command().args(first_arguments).output().unwrap();
    assert!(first.status.success());

    app.command()
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "2",
            "--title",
            "Second title",
            "--idempotency-key",
            "issue-edit-2",
        ])
        .assert()
        .success();

    let replay = app.command().args(first_arguments).output().unwrap();
    assert!(replay.status.success());
    assert_eq!(first.stdout, replay.stdout);
    assert_eq!(show_issue(&app)["revision"], 3);
    assert_eq!(show_issue(&app)["title"], "Second title");

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM domain_events WHERE event_type = 'issue_updated'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
}

#[test]
fn issue_edit_requires_a_revision_and_at_least_one_patch_field() {
    let app = initialized_issue();

    for arguments in [
        vec![
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--title",
            "Missing revision",
            "--json",
        ],
        vec![
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--json",
        ],
    ] {
        let output = app.command().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
    }

    assert_eq!(show_issue(&app)["revision"], 1);
}

#[test]
fn issue_edit_rejects_partial_or_mixed_assignee_patches() {
    let cases = [
        &["--assignee-kind", "agent"][..],
        &["--assignee-name", "codex"][..],
        &["--assignee-kind", "human", "--clear-assignee"][..],
        &["--assignee-name", "asonas", "--clear-assignee"][..],
    ];

    for patch in cases {
        let app = initialized_issue();
        let mut arguments = vec![
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
        ];
        arguments.extend_from_slice(patch);
        arguments.push("--json");

        let output = app.command().args(arguments).output().unwrap();

        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
        assert_eq!(show_issue(&app)["revision"], 1);
    }
}

#[test]
fn issue_edit_rejects_clearing_or_blank_title() {
    let app = initialized_issue();

    for patch in [&["--clear-title"][..], &["--title", "   "][..]] {
        let mut arguments = vec![
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
        ];
        arguments.extend_from_slice(patch);
        arguments.push("--json");
        let output = app.command().args(arguments).output().unwrap();

        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
    }

    assert_eq!(show_issue(&app)["title"], "Build local core");
}

#[test]
fn stale_issue_edit_reports_the_current_revision_without_changing_data() {
    let app = initialized_issue();
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
            "Current title",
        ])
        .assert()
        .success();
    let before = show_issue(&app);

    let stale = app
        .command()
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--title",
            "Stale title",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(stale.status.code(), Some(4));
    let response: serde_json::Value = serde_json::from_slice(&stale.stderr).unwrap();
    assert_eq!(response["error"]["code"], "revision_conflict");
    assert_eq!(response["error"]["details"]["current_revision"], 2);
    assert_eq!(show_issue(&app), before);
}
