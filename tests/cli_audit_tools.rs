mod support;

fn initialized_app() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command()
        .env("BETTR_OPERATOR", "initializer")
        .args(["init", "--json"])
        .assert()
        .success();
    app.command()
        .env("BETTR_OPERATOR", "project-owner")
        .args(["project", "create", "bettr", "--json"])
        .assert()
        .success();
    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "audit-tools-session")
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "audit tools fixture",
            "--json",
        ])
        .assert()
        .success();
    app
}

fn jsonl_path(app: &crate::support::TestApp) -> std::path::PathBuf {
    app.database.with_extension("audit.jsonl")
}

fn events_at(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn error_response(output: &std::process::Output) -> serde_json::Value {
    assert!(!output.status.success());
    serde_json::from_slice(&output.stderr).unwrap()
}

#[test]
fn verify_accepts_active_and_archived_generations() {
    let app = initialized_app();
    let active_path = jsonl_path(&app);
    let initial_events = events_at(&active_path);

    let verify = app
        .command()
        .args(["audit", "verify", "--json"])
        .output()
        .unwrap();
    assert!(verify.status.success());
    let response: serde_json::Value = serde_json::from_slice(&verify.stdout).unwrap();
    assert_eq!(response["data"]["valid"], true);
    assert_eq!(response["data"]["event_count"], initial_events.len());
    assert_eq!(response["data"]["first_sequence"], 1);

    let archive = app
        .command()
        .args(["audit", "archive", "--json"])
        .output()
        .unwrap();
    assert!(archive.status.success());
    let archive_response: serde_json::Value = serde_json::from_slice(&archive.stdout).unwrap();
    assert_eq!(archive_response["data"]["archived"], true);
    let archive_path =
        std::path::PathBuf::from(archive_response["data"]["archive_path"].as_str().unwrap());
    assert!(archive_path.is_file());
    assert!(active_path.is_file());
    let archived_events = events_at(&archive_path);
    let active_events = events_at(&active_path);
    assert_eq!(active_events.len(), 1);
    assert_eq!(
        active_events[0]["previous_hash"],
        archived_events.last().unwrap()["hash"]
    );

    let verify_archive = app
        .command()
        .args([
            "audit",
            "verify",
            "--path",
            archive_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(verify_archive.status.success());
    let archive_verify_response: serde_json::Value =
        serde_json::from_slice(&verify_archive.stdout).unwrap();
    assert_eq!(archive_verify_response["data"]["valid"], true);
    assert_eq!(
        archive_verify_response["data"]["event_count"],
        initial_events.len() + 1
    );
}

#[test]
fn verify_rejects_hash_mutation_with_recovery_contract() {
    let app = initialized_app();
    let path = jsonl_path(&app);
    let mut contents = std::fs::read_to_string(&path).unwrap();
    let replacement = if contents.contains("\"hash\":\"0") {
        "\"hash\":\"1"
    } else {
        "\"hash\":\"0"
    };
    let hash_start = contents.find("\"hash\":\"").unwrap();
    let value_start = hash_start + "\"hash\":\"".len();
    contents.replace_range(value_start..value_start + 1, replacement[8..9].as_ref());
    std::fs::write(&path, contents).unwrap();

    let output = app
        .command()
        .args(["audit", "verify", "--json"])
        .output()
        .unwrap();
    let response = error_response(&output);
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(response["error"]["code"], "audit_integrity_failure");
    assert_eq!(
        response["error"]["details"]["recovery"],
        "preserve the affected JSONL and run bettr audit rebuild --json"
    );
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("audit JSONL integrity")
    );
}

#[test]
fn verify_rejects_an_incomplete_final_line() {
    let app = initialized_app();
    let path = jsonl_path(&app);
    let mut contents = std::fs::read(&path).unwrap();
    contents.pop();
    std::fs::write(&path, contents).unwrap();

    let output = app
        .command()
        .args(["audit", "verify", "--json"])
        .output()
        .unwrap();
    let response = error_response(&output);
    assert_eq!(response["error"]["code"], "audit_integrity_failure");
    assert_eq!(response["error"]["details"]["line"], 3);
}

#[test]
fn verify_rejects_an_empty_line_between_events() {
    let app = initialized_app();
    let path = jsonl_path(&app);
    let contents = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, contents.replacen('\n', "\n\n", 1)).unwrap();

    let output = app
        .command()
        .args(["audit", "verify", "--json"])
        .output()
        .unwrap();
    let response = error_response(&output);
    assert_eq!(response["error"]["code"], "audit_integrity_failure");
    assert_eq!(response["error"]["details"]["line"], 2);
}

#[test]
fn rebuild_restores_active_jsonl_from_sqlite_and_updates_cursor() {
    let app = initialized_app();
    let path = jsonl_path(&app);
    let source_count: i64 = rusqlite::Connection::open(&app.database)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
        .unwrap();
    std::fs::write(&path, b"{\"not\":\"an audit event\"}\n").unwrap();

    let output = app
        .command()
        .args(["audit", "rebuild", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"]["rebuilt"], true);
    assert_eq!(response["data"]["event_count"], source_count);
    assert_eq!(response["data"]["first_sequence"], 1);
    assert_eq!(response["data"]["last_sequence"], source_count);

    let events = events_at(&path);
    assert_eq!(events.len(), source_count as usize + 1);
    assert_eq!(events.last().unwrap()["operation"], "audit_rebuild");
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let (cursor_sequence, cursor_hash): (i64, String) = connection
        .query_row(
            "SELECT sequence, previous_hash FROM audit_jsonl_cursor WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?.unwrap())),
        )
        .unwrap();
    assert_eq!(cursor_sequence, events.len() as i64);
    assert_eq!(cursor_hash, events.last().unwrap()["hash"]);
    let metadata: serde_json::Value = connection
        .query_row(
            "SELECT metadata_json FROM audit_events
             WHERE operation = 'audit_rebuild' ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|value| serde_json::from_str(&value).unwrap())
        .unwrap();
    assert_eq!(metadata["event_count"], source_count);
    assert_eq!(metadata["first_sequence"], 1);
    assert_eq!(metadata["last_sequence"], source_count);
    assert_eq!(metadata["rebuilt"], true);
}

#[test]
fn rebuild_rejects_a_gap_without_replacing_active_jsonl() {
    let app = initialized_app();
    let path = jsonl_path(&app);
    let before = std::fs::read(&path).unwrap();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute("DELETE FROM audit_events WHERE sequence = 2", [])
        .unwrap();

    let output = app
        .command()
        .args(["audit", "rebuild", "--json"])
        .output()
        .unwrap();
    let response = error_response(&output);
    assert_eq!(response["error"]["code"], "audit_integrity_failure");
    let after = std::fs::read(&path).unwrap();
    assert!(after.starts_with(&before));
}

#[test]
fn archive_refuses_an_active_file_that_disagrees_with_the_cursor() {
    let app = initialized_app();
    let path = jsonl_path(&app);
    let mut events = events_at(&path);
    events.pop();
    let contents = events
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n")
        + "\n";
    std::fs::write(&path, contents).unwrap();

    let output = app
        .command()
        .args(["audit", "archive", "--json"])
        .output()
        .unwrap();
    let response = error_response(&output);
    assert_eq!(response["error"]["code"], "audit_integrity_failure");
    assert_eq!(
        response["error"]["details"]["recovery"],
        "preserve the affected JSONL and run bettr audit rebuild --json"
    );
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| { entry.file_name().to_string_lossy().contains("audit.") })
            .count(),
        1
    );
}
