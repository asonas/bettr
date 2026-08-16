mod support;

fn run_json(
    app: &crate::support::TestApp,
    arguments: &[&str],
    environment: &[(&str, &str)],
) -> serde_json::Value {
    let mut command = app.command();
    command.args(arguments).arg("--json");
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    assert!(response.get("data").is_some());
    response["data"].clone()
}

#[test]
fn phase_one_workflow_preserves_revisions_states_events_and_context() {
    let app = crate::support::TestApp::new();

    let initialized = run_json(&app, &["init"], &[("BETTR_OPERATOR", "phase-one-owner")]);
    assert_eq!(initialized["initialized"], true);

    let project = run_json(
        &app,
        &["project", "create", "bettr"],
        &[("BETTR_OPERATOR", "phase-one-owner")],
    );
    assert_eq!(project["name"], "bettr");

    let created = run_json(
        &app,
        &[
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Verify the Phase 1 workflow",
            "--priority",
            "critical",
        ],
        &[("BETTR_OPERATOR", "issue-author")],
    );
    assert_eq!(created["number"], 1);
    assert_eq!(created["state"], "todo");
    assert_eq!(created["priority"], "critical");
    assert_eq!(created["revision"], 1);

    let assigned = run_json(
        &app,
        &[
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--assignee-kind",
            "agent",
            "--assignee-name",
            "codex",
        ],
        &[
            ("BETTR_AGENT", "dispatcher"),
            ("BETTR_SESSION_ID", "session-1"),
        ],
    );
    assert_eq!(assigned["assignee_kind"], "agent");
    assert_eq!(assigned["assignee_name"], "codex");
    assert_eq!(assigned["state"], "todo");
    assert_eq!(assigned["revision"], 2);

    let started = run_json(
        &app,
        &[
            "issue",
            "start",
            "1",
            "--project",
            "bettr",
            "--revision",
            "2",
        ],
        &[("BETTR_AGENT", "codex"), ("BETTR_SESSION_ID", "session-1")],
    );
    assert_eq!(started["state"], "in_progress");
    assert_eq!(started["revision"], 3);

    let comment = run_json(
        &app,
        &[
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "Implementation is ready for review",
        ],
        &[("BETTR_AGENT", "codex"), ("BETTR_SESSION_ID", "session-1")],
    );
    assert_eq!(comment["body"], "Implementation is ready for review");
    assert_eq!(comment["context"]["kind"], "agent");
    assert_eq!(comment["context"]["agent"], "codex");
    assert_eq!(comment["context"]["session_id"], "session-1");

    let blocked = run_json(
        &app,
        &[
            "issue",
            "block",
            "1",
            "--project",
            "bettr",
            "--revision",
            "3",
            "--reason",
            "Waiting for review",
            "--wait-kind",
            "human",
        ],
        &[("BETTR_AGENT", "codex"), ("BETTR_SESSION_ID", "session-1")],
    );
    assert_eq!(blocked["state"], "blocked");
    assert_eq!(blocked["revision"], 4);

    let resumed = run_json(
        &app,
        &[
            "issue",
            "resume",
            "1",
            "--project",
            "bettr",
            "--revision",
            "4",
        ],
        &[("BETTR_OPERATOR", "reviewer")],
    );
    assert_eq!(resumed["state"], "in_progress");
    assert_eq!(resumed["revision"], 5);

    let completed = run_json(
        &app,
        &[
            "issue",
            "complete",
            "1",
            "--project",
            "bettr",
            "--revision",
            "5",
            "--summary",
            "Phase 1 workflow verified",
            "--verification",
            "cargo test passed",
        ],
        &[("BETTR_AGENT", "codex"), ("BETTR_SESSION_ID", "session-1")],
    );
    assert_eq!(completed["state"], "done");
    assert_eq!(completed["revision"], 6);

    let history = run_json(
        &app,
        &["issue", "history", "1", "--project", "bettr"],
        &[("BETTR_OPERATOR", "auditor")],
    );
    let events = history.as_array().unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event["event_type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "issue_created",
            "issue_updated",
            "issue_started",
            "comment_added",
            "issue_blocked",
            "issue_resumed",
            "issue_completed",
        ]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["revision"].as_i64())
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3), None, Some(4), Some(5), Some(6)]
    );
    assert!(events.windows(2).all(|pair| {
        pair[0]["sequence"].as_i64().unwrap() < pair[1]["sequence"].as_i64().unwrap()
    }));
    assert_eq!(events[0]["context"]["operator"], "issue-author");
    assert_eq!(events[1]["context"]["agent"], "dispatcher");
    assert_eq!(events[1]["context"]["session_id"], "session-1");
    assert_eq!(events[2]["context"]["agent"], "codex");
    assert_eq!(events[3]["context"]["agent"], "codex");
    assert_eq!(events[4]["metadata"]["reason"], "Waiting for review");
    assert_eq!(events[4]["metadata"]["wait_kind"], "human");
    assert_eq!(events[5]["context"]["operator"], "reviewer");
    assert_eq!(
        events[6]["metadata"]["summary"],
        "Phase 1 workflow verified"
    );

    let status = run_json(&app, &["status"], &[("BETTR_OPERATOR", "auditor")]);
    assert_eq!(status["blocked"], serde_json::json!([]));
    assert_eq!(status["recently_completed"][0]["project"], "bettr");
    assert_eq!(status["recently_completed"][0]["number"], 1);
    assert_eq!(status["recently_completed"][0]["state"], "done");
    assert_eq!(status["recently_completed"][0]["revision"], 6);

    let audit = run_json(&app, &["audit", "list"], &[("BETTR_OPERATOR", "auditor")]);
    let audit_events = audit.as_array().unwrap();
    let workflow_operations = audit_events
        .iter()
        .map(|event| event["operation"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        workflow_operations,
        vec![
            "init",
            "project_create",
            "issue_create",
            "issue_edit",
            "issue_start",
            "issue_comment",
            "issue_block",
            "issue_resume",
            "issue_complete",
            "issue_history",
            "status",
        ]
    );
    for event in audit_events {
        assert_eq!(event["outcome"], "success");
        assert_eq!(event["exit_code"], 0);
        assert!(event["context"]["kind"] == "human" || event["context"]["kind"] == "agent");
    }
    let issue_audit = audit_events
        .iter()
        .find(|event| event["operation"] == "issue_complete")
        .unwrap();
    assert_eq!(issue_audit["project"]["name"], "bettr");
    assert_eq!(issue_audit["target"]["kind"], "issue");
    assert_eq!(issue_audit["context"]["agent"], "codex");
    assert_eq!(issue_audit["context"]["session_id"], "session-1");
    assert_eq!(issue_audit["revision"], 6);
}
