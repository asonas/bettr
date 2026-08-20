mod support;

fn initialized_project() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    app
}

fn write_batch(app: &crate::support::TestApp, content: &str) -> std::path::PathBuf {
    let path = app.dir.path().join("batch.json");
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn issue_batch_commits_all_operations_and_replays_the_same_result() {
    let app = initialized_project();
    let input = write_batch(
        &app,
        r#"[
          {"operation":"issue_create","title":"First issue"},
          {"operation":"issue_edit","number":1,"revision":1,"patch":{"title":"Updated issue"}}
        ]"#,
    );
    let arguments = [
        "issue",
        "batch",
        "--input",
        input.to_str().unwrap(),
        "--project",
        "bettr",
        "--idempotency-key",
        "batch-1",
        "--json",
    ];

    let first = app.command().args(arguments).output().unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = app.command().args(arguments).output().unwrap();
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);

    let response: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(response["data"].as_array().unwrap().len(), 2);
    assert_eq!(response["data"][0]["operation"], "issue_create");
    assert_eq!(response["data"][1]["operation"], "issue_edit");
    assert_eq!(response["data"][1]["result"]["title"], "Updated issue");

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM issues", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM domain_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE operation = 'issue_batch' AND success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn issue_batch_rolls_back_earlier_operations_when_a_later_operation_fails() {
    let app = initialized_project();
    let input = write_batch(
        &app,
        r#"[
          {"operation":"issue_create","title":"Must roll back"},
          {"operation":"issue_edit","number":1,"revision":99,"patch":{"title":"Never saved"}}
        ]"#,
    );

    app.command()
        .args([
            "issue",
            "batch",
            "--input",
            input.to_str().unwrap(),
            "--project",
            "bettr",
            "--json",
        ])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("revision_conflict"));

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
            .query_row("SELECT COUNT(*) FROM domain_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE operation = 'issue_batch' AND success = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}
