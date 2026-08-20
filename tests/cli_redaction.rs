mod support;

use serde_json::Value;

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

fn response_data(output: &[u8]) -> Value {
    serde_json::from_slice::<Value>(output).unwrap()["data"].clone()
}

fn all_text(connection: &rusqlite::Connection, query: &str) -> Vec<String> {
    let mut statement = connection.prepare(query).unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn issue_redaction_removes_related_content_and_preserves_the_jsonl_prefix() {
    let app = initialized_app();
    let secret_title = "issue-title-secret";
    let secret_body = "issue-body-secret";
    let secret_edit_title = "edit-title-secret";
    let secret_edit_body = "edit-body-secret";
    let secret_comment = "comment-body-secret";

    app.command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "redaction-fixture")
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            secret_title,
            "--body",
            secret_body,
            "--idempotency-key",
            "issue-create-key",
            "--json",
        ])
        .assert()
        .success();
    app.command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "redaction-fixture")
        .args([
            "issue",
            "edit",
            "1",
            "--revision",
            "1",
            "--project",
            "bettr",
            "--title",
            secret_edit_title,
            "--body",
            secret_edit_body,
            "--idempotency-key",
            "issue-edit-key",
            "--json",
        ])
        .assert()
        .success();
    let comment_output = app
        .command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "redaction-fixture")
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            secret_comment,
            "--idempotency-key",
            "comment-key",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let comment_id = response_data(&comment_output)["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let jsonl_path = app.database.with_extension("audit.jsonl");
    let before_redaction = std::fs::read_to_string(&jsonl_path).unwrap();

    let redaction_output = app
        .command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args(["redact", "issue", "1", "--project", "bettr", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let redaction = response_data(&redaction_output);
    assert_eq!(redaction["target_type"], "issue");
    assert!(redaction["target_id"].is_string());
    assert!(redaction["changed_count"].as_u64().unwrap() >= 5);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let issue_values = all_text(
        &connection,
        "SELECT title || COALESCE(body, '') FROM issues",
    );
    assert_eq!(issue_values, vec!["[REDACTED][REDACTED]"]);
    let comment_values = all_text(&connection, "SELECT body || metadata_json FROM comments");
    assert_eq!(comment_values, vec!["[REDACTED]{\"redacted\":true}"]);
    for query in [
        "SELECT metadata_json FROM domain_events",
        "SELECT request_hash || response_json FROM idempotency_records",
        "SELECT COALESCE(idempotency_key, '') || metadata_json FROM audit_events",
    ] {
        for value in all_text(&connection, query) {
            for secret in [
                secret_title,
                secret_body,
                secret_edit_title,
                secret_edit_body,
                secret_comment,
            ] {
                assert!(!value.contains(secret), "redaction leaked {secret}");
            }
        }
    }
    let redacted_domain_event = connection
        .query_row(
            "SELECT metadata_json FROM domain_events
             WHERE event_type = 'comment_added'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    let redacted_domain_event: Value = serde_json::from_str(&redacted_domain_event).unwrap();
    assert_eq!(redacted_domain_event["comment_id"], comment_id);
    assert_eq!(redacted_domain_event["redacted"], true);
    assert!(redacted_domain_event["context"]["kind"].is_string());
    let audit_keys = connection
        .prepare(
            "SELECT COUNT(*) FROM audit_events
             WHERE target_type = 'issue' AND idempotency_key IS NOT NULL",
        )
        .unwrap()
        .query_row([], |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(audit_keys, 0);
    drop(connection);

    let after_redaction = std::fs::read_to_string(&jsonl_path).unwrap();
    assert!(after_redaction.starts_with(&before_redaction));
    app.command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args(["audit", "verify", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains(r#""valid":true"#));
    let shown = app
        .command()
        .env("BETTR_OPERATOR", "reader")
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown = response_data(&shown);
    assert_eq!(shown["title"], "[REDACTED]");
    assert_eq!(shown["body"], "[REDACTED]");
    let history = app
        .command()
        .env("BETTR_OPERATOR", "reader")
        .args(["issue", "history", "1", "--project", "bettr", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let history = String::from_utf8(history).unwrap();
    for secret in [
        secret_title,
        secret_body,
        secret_edit_title,
        secret_edit_body,
        secret_comment,
    ] {
        assert!(!history.contains(secret), "history leaked {secret}");
    }

    let repeat_output = app
        .command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args(["redact", "issue", "1", "--project", "bettr", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(response_data(&repeat_output)["changed_count"], 0);
}

#[test]
fn agent_redaction_is_rejected_without_changing_the_issue() {
    let app = initialized_app();
    app.command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "redaction-agent-fixture")
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "agent-redaction-title",
            "--body",
            "agent-redaction-body",
            "--json",
        ])
        .assert()
        .success();

    app.command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "redaction-agent-fixture")
        .args(["redact", "issue", "1", "--project", "bettr", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("redaction requires a human"));

    let shown = app
        .command()
        .env("BETTR_OPERATOR", "reader")
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown = response_data(&shown);
    assert_eq!(shown["title"], "agent-redaction-title");
    assert_eq!(shown["body"], "agent-redaction-body");
}

#[test]
fn comment_and_audit_redaction_use_explicit_targets() {
    let app = initialized_app();
    let comment_secret = "comment-target-secret";
    let audit_secret = "audit-target-secret";
    app.command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "target-fixture")
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "target issue",
            "--body",
            "target body",
            "--json",
        ])
        .assert()
        .success();
    let comment_output = app
        .command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "target-fixture")
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            comment_secret,
            "--idempotency-key",
            "target-comment-key",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let comment_id = response_data(&comment_output)["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let audit_id = connection
        .query_row(
            "SELECT id FROM audit_events
             WHERE operation = 'issue_comment'
             ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    drop(connection);

    let comment_redaction = app
        .command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args(["redact", "comment", &comment_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let comment_redaction = response_data(&comment_redaction);
    assert_eq!(comment_redaction["target_type"], "comment");
    assert_eq!(comment_redaction["target_id"], comment_id);
    assert!(comment_redaction["changed_count"].as_u64().unwrap() >= 2);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let comment_record = connection
        .query_row(
            "SELECT body, metadata_json FROM comments WHERE id = ?1",
            [&comment_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap();
    assert_eq!(
        comment_record,
        ("[REDACTED]".to_owned(), r#"{"redacted":true}"#.to_owned())
    );
    connection
        .execute(
            "UPDATE audit_events
             SET idempotency_key = ?1, metadata_json = ?2 WHERE id = ?3",
            rusqlite::params![
                "audit-target-key",
                serde_json::json!({"secret": audit_secret}).to_string(),
                audit_id
            ],
        )
        .unwrap();
    drop(connection);

    let audit_redaction = app
        .command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args(["redact", "audit", &audit_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let audit_redaction = response_data(&audit_redaction);
    assert_eq!(audit_redaction["target_type"], "audit");
    assert_eq!(audit_redaction["target_id"], audit_id);
    assert_eq!(audit_redaction["changed_count"], 1);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let redacted_audit = connection
        .query_row(
            "SELECT idempotency_key, metadata_json FROM audit_events WHERE id = ?1",
            [&audit_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap();
    assert_eq!(redacted_audit, (None, r#"{"redacted":true}"#.to_owned()));
    for query in [
        "SELECT body || metadata_json FROM comments",
        "SELECT metadata_json FROM domain_events",
        "SELECT request_hash || response_json FROM idempotency_records",
        "SELECT COALESCE(idempotency_key, '') || metadata_json FROM audit_events",
    ] {
        for value in all_text(&connection, query) {
            assert!(!value.contains(comment_secret));
            assert!(!value.contains(audit_secret));
        }
    }
    drop(connection);

    app.command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args([
            "redact",
            "comment",
            "00000000-0000-4000-8000-000000000000",
            "--json",
        ])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("comment not found"));
    app.command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args(["audit", "verify", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains(r#""valid":true"#));
}

#[test]
fn failed_domain_metadata_redaction_rolls_back_all_changes() {
    let app = initialized_app();
    app.command()
        .env("BETTR_AGENT", "writer")
        .env("BETTR_SESSION_ID", "rollback-fixture")
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "rollback-title",
            "--body",
            "rollback-body",
            "--json",
        ])
        .assert()
        .success();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute(
            "UPDATE domain_events SET metadata_json = ?1 WHERE event_type = 'issue_created'",
            ["not-json"],
        )
        .unwrap();
    drop(connection);

    app.command()
        .env("BETTR_OPERATOR", "privacy-operator")
        .args(["redact", "issue", "1", "--project", "bettr", "--json"])
        .assert()
        .code(10)
        .stderr(predicates::str::contains("internal_error"));

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let issue = connection
        .query_row(
            "SELECT title, body FROM issues WHERE number = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap();
    assert_eq!(
        issue,
        ("rollback-title".to_owned(), "rollback-body".to_owned())
    );
    let metadata = connection
        .query_row(
            "SELECT metadata_json FROM domain_events WHERE event_type = 'issue_created'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(metadata, "not-json");
}
