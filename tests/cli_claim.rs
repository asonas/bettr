mod support;

fn initialized_app() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    for title in ["First claimable Issue", "Second claimable Issue"] {
        app.command()
            .args(["issue", "create", "--project", "bettr", "--title", title])
            .assert()
            .success();
    }
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
fn claim_heartbeat_stale_and_takeover_follow_lease_ownership() {
    let app = initialized_app();

    let claimed = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "claim", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    let claimed = json_data(&claimed);
    assert_eq!(claimed["issue"]["state"], "in_progress");
    assert_eq!(claimed["issue"]["assignee_name"], "codex");
    assert_eq!(claimed["lease"]["agent"], "codex");
    assert_eq!(claimed["lease"]["session_id"], "session-a");
    assert!(claimed["lease"]["expires_at"].as_str().is_some());

    app.command()
        .env("BETTR_AGENT", "other-agent")
        .env("BETTR_SESSION_ID", "session-b")
        .args(["issue", "claim", "1", "--project", "bettr", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));

    app.command()
        .env("BETTR_AGENT", "other-agent")
        .env("BETTR_SESSION_ID", "session-b")
        .args(["issue", "heartbeat", "1", "--project", "bettr", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));

    let heartbeat = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "heartbeat", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    let heartbeat = json_data(&heartbeat);
    assert_eq!(heartbeat["issue"]["revision"], claimed["issue"]["revision"]);
    assert_eq!(heartbeat["lease"]["session_id"], "session-a");

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute(
            "UPDATE issue_leases SET expires_at = '2000-01-01T00:00:00Z' WHERE issue_id = (
                 SELECT id FROM issues WHERE number = 1
             )",
            [],
        )
        .unwrap();
    drop(connection);

    let status = app.command().args(["status", "--json"]).output().unwrap();
    let status = json_data(&status);
    assert_eq!(status["stale"][0]["number"], 1);
    assert_eq!(status["stale"][0]["state"], "in_progress");

    let takeover = app
        .command()
        .env("BETTR_AGENT", "other-agent")
        .env("BETTR_SESSION_ID", "session-b")
        .args([
            "issue",
            "takeover",
            "1",
            "--project",
            "bettr",
            "--reason",
            "Previous session expired",
            "--json",
        ])
        .output()
        .unwrap();
    let takeover = json_data(&takeover);
    assert_eq!(takeover["issue"]["state"], "in_progress");
    assert_eq!(takeover["issue"]["assignee_name"], "other-agent");
    assert_eq!(takeover["lease"]["session_id"], "session-b");
    assert!(
        takeover["issue"]["revision"].as_i64().unwrap()
            > claimed["issue"]["revision"].as_i64().unwrap()
    );
}

#[test]
fn automatic_claim_selects_the_first_eligible_issue() {
    let app = initialized_app();

    let claimed = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "claim", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    let claimed = json_data(&claimed);
    assert_eq!(claimed["issue"]["number"], 1);
}

#[test]
fn completing_and_reopening_a_claimed_issue_releases_its_lease() {
    let app = initialized_app();

    let claimed = app
        .command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "claim", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    let claimed = json_data(&claimed);
    assert_eq!(claimed["issue"]["revision"], 2);

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
            "Implemented",
            "--verification",
            "Tests passed",
            "--json",
        ])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM issue_leases", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    drop(connection);

    app.command()
        .args([
            "issue",
            "reopen",
            "1",
            "--project",
            "bettr",
            "--revision",
            "3",
            "--reason",
            "Work is needed again",
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
fn takeover_restores_in_progress_for_a_stale_todo_issue() {
    let app = initialized_app();

    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "session-a")
        .args(["issue", "claim", "1", "--project", "bettr", "--json"])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute("UPDATE issues SET state = 'todo' WHERE number = 1", [])
        .unwrap();
    connection
        .execute(
            "UPDATE issue_leases SET expires_at = '2000-01-01T00:00:00Z'",
            [],
        )
        .unwrap();
    drop(connection);

    let takeover = app
        .command()
        .env("BETTR_AGENT", "worker")
        .env("BETTR_SESSION_ID", "session-b")
        .args([
            "issue",
            "takeover",
            "1",
            "--project",
            "bettr",
            "--reason",
            "Previous session expired",
            "--json",
        ])
        .output()
        .unwrap();
    let takeover = json_data(&takeover);
    assert_eq!(takeover["issue"]["state"], "in_progress");
    assert_eq!(takeover["lease"]["session_id"], "session-b");
}
