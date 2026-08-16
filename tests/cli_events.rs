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
            "Cursor source",
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
fn event_cursor_is_exclusive_and_heartbeat_is_not_a_domain_event() {
    let app = initialized_app();
    app.command()
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "first comment",
        ])
        .assert()
        .success();

    let first_page = app
        .command()
        .args([
            "event",
            "list",
            "--after",
            "0",
            "--limit",
            "2",
            "--include-issue",
            "--json",
        ])
        .output()
        .unwrap();
    let first_page = json_data(&first_page);
    assert_eq!(first_page["events"].as_array().unwrap().len(), 2);
    assert_eq!(first_page["events"][0]["sequence"], 1);
    assert_eq!(first_page["events"][0]["event_type"], "project_created");
    assert_eq!(first_page["events"][0]["issue"], serde_json::Value::Null);
    assert_eq!(first_page["events"][1]["event_type"], "issue_created");
    assert!(first_page["events"][1]["issue"]["id"].as_str().is_some());
    assert_eq!(first_page["has_more"], true);
    assert_eq!(first_page["next_cursor"], 2);

    let second_page = app
        .command()
        .args(["event", "list", "--after", "2", "--limit", "50", "--json"])
        .output()
        .unwrap();
    let second_page = json_data(&second_page);
    let sequences = second_page["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["sequence"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert!(sequences.iter().all(|sequence| *sequence > 2));
    assert!(
        second_page["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["event_type"] != "issue_heartbeat")
    );
    assert_eq!(second_page["has_more"], false);
    assert_eq!(
        second_page["next_cursor"],
        sequences.last().copied().unwrap()
    );

    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "claim", "1", "--project", "bettr", "--json"])
        .assert()
        .success();
    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "heartbeat", "1", "--project", "bettr", "--json"])
        .assert()
        .success();

    let after_claim = app
        .command()
        .args([
            "event",
            "list",
            "--after",
            &second_page["next_cursor"].to_string(),
            "--json",
        ])
        .output()
        .unwrap();
    let after_claim = json_data(&after_claim);
    assert!(
        after_claim["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event_type"] == "issue_claimed")
    );
    assert!(
        after_claim["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["event_type"] != "issue_heartbeat")
    );
}

#[test]
fn event_cursor_validates_after_and_limit_and_returns_empty_page() {
    let app = initialized_app();

    app.command()
        .args(["event", "list", "--after", "-1", "--json"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));
    app.command()
        .args(["event", "list", "--after", "0", "--limit", "0", "--json"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));

    let page = app
        .command()
        .args(["event", "list", "--after", "999", "--json"])
        .output()
        .unwrap();
    let page = json_data(&page);
    assert_eq!(page["events"], serde_json::json!([]));
    assert_eq!(page["next_cursor"], 999);
    assert_eq!(page["has_more"], false);
}
