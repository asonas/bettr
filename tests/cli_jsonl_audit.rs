mod support;

use std::io::Write as _;

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
    app
}

#[test]
fn audit_rows_have_a_contiguous_source_sequence() {
    let app = initialized_app();
    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "jsonl-session")
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "safe title",
            "--body",
            "safe body",
            "--json",
        ])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let sequences = connection
        .prepare("SELECT sequence FROM audit_events ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(sequences, (1..=sequences.len() as i64).collect::<Vec<_>>());
}

#[test]
fn cli_projects_safe_hashed_audit_events_to_jsonl() {
    let app = initialized_app();
    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "jsonl-session")
        .env("JSONL_TEST_SECRET", "environment-secret")
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "private title",
            "--body",
            "private body",
            "--json",
        ])
        .assert()
        .success();

    let log_path = app.database.with_extension("audit.jsonl");
    let contents = std::fs::read_to_string(log_path).unwrap();
    let events = contents
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(events.len() >= 3);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["schema_version"], 1);
        assert_eq!(event["sequence"], (index + 1) as i64);
        assert!(event["event_id"].is_string());
        assert!(event["operation"].is_string());
        assert!(event["context"]["kind"].is_string());
        assert!(event["result"]["outcome"] == "success");
        assert_eq!(event["hash"].as_str().unwrap().len(), 64);
        let mut hash_input = event.clone();
        let hash = hash_input.as_object_mut().unwrap().remove("hash").unwrap();
        let digest =
            <sha2::Sha256 as sha2::Digest>::digest(serde_json::to_vec(&hash_input).unwrap());
        let expected_hash = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(hash.as_str().unwrap(), expected_hash);
        if index > 0 {
            assert_eq!(event["previous_hash"], events[index - 1]["hash"]);
        } else {
            assert!(event["previous_hash"].is_null());
        }
    }
    let serialized = contents;
    for forbidden in [
        "private title",
        "private body",
        "environment-secret",
        "JSONL_TEST_SECRET",
    ] {
        assert!(!serialized.contains(forbidden), "JSONL leaked {forbidden}");
    }
}

fn jsonl_events(app: &crate::support::TestApp) -> Vec<serde_json::Value> {
    std::fs::read_to_string(app.database.with_extension("audit.jsonl"))
        .unwrap()
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn jsonl_recovers_a_partial_tail_and_does_not_duplicate_a_complete_tail() {
    let app = initialized_app();
    let log_path = app.database.with_extension("audit.jsonl");
    let initial = jsonl_events(&app);
    let initial_len = initial.len();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .unwrap()
        .write_all(b"{\"partial\":")
        .unwrap();

    app.command().args(["status", "--json"]).assert().success();
    let after_partial = jsonl_events(&app);
    assert_eq!(after_partial.len(), initial_len + 1);
    assert_eq!(
        after_partial.last().unwrap()["sequence"],
        (initial_len + 1) as i64
    );

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute(
            "UPDATE audit_jsonl_cursor SET sequence = ?1, previous_hash = ?2 WHERE id = 1",
            rusqlite::params![
                (initial_len - 1) as i64,
                after_partial[initial_len - 2]["hash"].as_str().unwrap(),
            ],
        )
        .unwrap();
    app.command().args(["status", "--json"]).assert().success();

    let after_duplicate = jsonl_events(&app);
    assert_eq!(after_duplicate.len(), initial_len + 2);
    let sequences = after_duplicate
        .iter()
        .map(|event| event["sequence"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        sequences,
        (1..=(initial_len + 2) as i64).collect::<Vec<_>>()
    );
}

#[test]
fn jsonl_rotation_starts_a_new_file_from_the_previous_hash() {
    let app = initialized_app();
    let log_path = app.database.with_extension("audit.jsonl");
    let old_events = jsonl_events(&app);
    let last_sequence = old_events.last().unwrap()["sequence"].as_i64().unwrap();
    let last_hash = old_events.last().unwrap()["hash"].clone();
    let archive_path = app.database.with_extension("audit.previous.jsonl");
    std::fs::rename(&log_path, &archive_path).unwrap();

    app.command().args(["status", "--json"]).assert().success();

    let new_events = jsonl_events(&app);
    assert_eq!(new_events.len(), 1);
    assert_eq!(new_events[0]["sequence"], last_sequence + 1);
    assert_eq!(new_events[0]["previous_hash"], last_hash);
    assert!(archive_path.is_file());
}

#[test]
fn concurrent_cli_processes_append_without_duplicate_sequences() {
    let app = initialized_app();
    let database = app.database.clone();
    let root = app.dir.path().to_path_buf();
    std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|index| {
                let database = database.clone();
                let root = root.clone();
                scope.spawn(move || {
                    let mut command = assert_cmd::Command::cargo_bin("bettr").unwrap();
                    let output = command
                        .arg("--database")
                        .arg(database)
                        .env_clear()
                        .current_dir(root.join("work"))
                        .env("HOME", root.join("home"))
                        .env("XDG_CONFIG_HOME", root.join("config"))
                        .env("XDG_DATA_HOME", root.join("data"))
                        .env("BETTR_AGENT", format!("reader-{index}"))
                        .env("BETTR_SESSION_ID", format!("concurrent-{index}"))
                        .args(["status", "--json"])
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "concurrent status failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
    });

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let audit_count = connection
        .query_row("SELECT COUNT(*) FROM audit_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(audit_count, 10);
    let events = jsonl_events(&app);
    assert_eq!(events.len(), 10);
    let sequences = events
        .iter()
        .map(|event| event["sequence"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(sequences, (1..=10).collect::<Vec<_>>());
}

#[test]
fn jsonl_contains_safe_failure_results_for_reads_and_writes() {
    let app = initialized_app();
    app.command()
        .args(["issue", "show", "404", "--project", "bettr", "--json"])
        .assert()
        .code(3);
    app.command()
        .args(["project", "create", "bettr", "--json"])
        .assert()
        .code(4);
    app.command()
        .args([
            "issue",
            "show",
            "argv-secret-token",
            "--project",
            "bettr",
            "--json",
        ])
        .assert()
        .code(2);

    let events = jsonl_events(&app);
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(!serialized.contains("argv-secret-token"));
    let failures = events
        .into_iter()
        .filter(|event| event["result"]["outcome"] == "failure")
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 3);
    assert!(failures.iter().all(|event| {
        event["result"]["error_code"].is_string() && event["result"]["message"].is_null()
    }));
}
