mod support;

fn create_project(app: &crate::support::TestApp, name: &str) {
    app.command()
        .args(["project", "create", name])
        .assert()
        .success();
}

fn create_issue(
    app: &crate::support::TestApp,
    project: &str,
    title: &str,
    body: Option<&str>,
    priority: Option<&str>,
) -> String {
    let mut command = app.command();
    command.args([
        "issue",
        "create",
        "--project",
        project,
        "--title",
        title,
        "--json",
    ]);
    if let Some(body) = body {
        command.args(["--body", body]);
    }
    if let Some(priority) = priority {
        command.args(["--priority", priority]);
    }
    let output = command.output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn set_issue_fields(
    app: &crate::support::TestApp,
    id: &str,
    state: &str,
    assignee: Option<&str>,
    created_at: &str,
    updated_at: &str,
) {
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute(
            "UPDATE issues
             SET state = ?1, assignee_kind = ?2, assignee_name = ?3,
                 created_at = ?4, updated_at = ?5
             WHERE id = ?6",
            rusqlite::params![
                state,
                assignee.map(|_| "agent"),
                assignee,
                created_at,
                updated_at,
                id
            ],
        )
        .unwrap();
}

fn json_data(output: &std::process::Output) -> serde_json::Value {
    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    response["data"].clone()
}

fn titles(data: &serde_json::Value) -> Vec<&str> {
    data.as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["title"].as_str().unwrap())
        .collect()
}

#[test]
fn issue_list_requires_a_project_by_default_and_excludes_completed_issues() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");
    create_project(&app, "beta");

    let todo = create_issue(&app, "alpha", "Alpha todo", None, Some("low"));
    let done = create_issue(&app, "alpha", "Alpha done", None, Some("high"));
    let cancelled = create_issue(&app, "alpha", "Alpha cancelled", None, None);
    create_issue(&app, "beta", "Beta todo", None, Some("critical"));
    set_issue_fields(
        &app,
        &todo,
        "todo",
        None,
        "2026-08-10T00:00:00Z",
        "2026-08-10T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &done,
        "done",
        None,
        "2026-08-11T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &cancelled,
        "cancelled",
        None,
        "2026-08-12T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );

    app.command()
        .args(["issue", "list", "--json"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("invalid_input"));

    let output = app
        .command()
        .args(["issue", "list", "--project", "alpha", "--json"])
        .output()
        .unwrap();
    let data = json_data(&output);
    assert_eq!(data.as_array().unwrap().len(), 1);
    assert_eq!(data[0]["project"], "alpha");
    assert_eq!(data[0]["title"], "Alpha todo");

    app.command()
        .args(["issue", "list", "--project", "missing", "--json"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("not_found"));

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let audits = connection
        .prepare(
            "SELECT success, metadata_json FROM audit_events
             WHERE operation = 'issue_list' ORDER BY occurred_at, rowid",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        audits,
        [
            (0, r#"{"error_code":"invalid_input"}"#.to_owned()),
            (1, "{}".to_owned()),
            (0, r#"{"error_code":"not_found"}"#.to_owned())
        ]
    );
}

#[test]
fn issue_list_filters_across_projects_and_includes_completed_on_request() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");
    create_project(&app, "beta");

    let alpha_todo = create_issue(
        &app,
        "alpha",
        "Literal %_ marker",
        Some("searchable body"),
        Some("low"),
    );
    let alpha_blocked = create_issue(
        &app,
        "alpha",
        "Blocked work",
        Some("needs a decision"),
        Some("high"),
    );
    let alpha_progress = create_issue(
        &app,
        "alpha",
        "Progress work",
        Some("needle in body"),
        Some("critical"),
    );
    let alpha_done = create_issue(&app, "alpha", "Done work", None, Some("medium"));
    let beta_todo = create_issue(&app, "beta", "Needle in title", None, Some("high"));

    set_issue_fields(
        &app,
        &alpha_todo,
        "todo",
        Some("alice"),
        "2026-08-10T00:00:00Z",
        "2026-08-10T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &alpha_blocked,
        "blocked",
        Some("alice"),
        "2026-08-12T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &alpha_progress,
        "in_progress",
        Some("bob"),
        "2026-08-11T00:00:00Z",
        "2026-08-14T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &alpha_done,
        "done",
        Some("alice"),
        "2026-08-09T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &beta_todo,
        "todo",
        Some("alice"),
        "2026-08-08T00:00:00Z",
        "2026-08-13T00:00:00Z",
    );

    let all = app
        .command()
        .args(["issue", "list", "--all-projects", "--json"])
        .output()
        .unwrap();
    let all = json_data(&all);
    let ordered_titles = all
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["title"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        ordered_titles,
        [
            "Blocked work",
            "Progress work",
            "Needle in title",
            "Literal %_ marker"
        ]
    );

    let states = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--state",
            "todo",
            "--state",
            "blocked",
            "--json",
        ])
        .output()
        .unwrap();
    let states = json_data(&states);
    assert_eq!(states.as_array().unwrap().len(), 3);

    let in_progress = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--state",
            "in_progress",
            "--json",
        ])
        .output()
        .unwrap();
    let in_progress = json_data(&in_progress);
    assert_eq!(titles(&in_progress), ["Progress work"]);

    let priorities = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--priority",
            "high",
            "--priority",
            "critical",
            "--json",
        ])
        .output()
        .unwrap();
    let priorities = json_data(&priorities);
    assert_eq!(priorities.as_array().unwrap().len(), 3);

    let updated_after = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--updated-after",
            "2026-08-14T12:00:00Z",
            "--json",
        ])
        .output()
        .unwrap();
    let updated_after = json_data(&updated_after);
    assert_eq!(titles(&updated_after), ["Blocked work"]);

    let assignee = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--assignee",
            "bob",
            "--json",
        ])
        .output()
        .unwrap();
    let assignee = json_data(&assignee);
    assert_eq!(titles(&assignee), ["Progress work"]);

    let completed = app
        .command()
        .args([
            "issue",
            "list",
            "--project",
            "alpha",
            "--include-completed",
            "--json",
        ])
        .output()
        .unwrap();
    let completed = json_data(&completed);
    assert_eq!(completed.as_array().unwrap().len(), 4);

    let body_query = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--query",
            "needle",
            "--json",
        ])
        .output()
        .unwrap();
    let body_query = json_data(&body_query);
    assert_eq!(body_query.as_array().unwrap().len(), 2);

    let literal_query = app
        .command()
        .args(["issue", "list", "--all-projects", "--query", "%_", "--json"])
        .output()
        .unwrap();
    let literal_query = json_data(&literal_query);
    assert_eq!(literal_query.as_array().unwrap().len(), 1);
    assert_eq!(literal_query[0]["title"], "Literal %_ marker");
}

#[test]
fn issue_list_orders_state_rank_before_priority() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");

    let todo = create_issue(&app, "alpha", "Todo critical", None, Some("critical"));
    let progress = create_issue(&app, "alpha", "Progress critical", None, Some("critical"));
    let blocked = create_issue(&app, "alpha", "Blocked low", None, Some("low"));
    set_issue_fields(
        &app,
        &todo,
        "todo",
        None,
        "2026-08-08T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &progress,
        "in_progress",
        None,
        "2026-08-09T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &blocked,
        "blocked",
        None,
        "2026-08-10T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );

    let output = app
        .command()
        .args(["issue", "list", "--all-projects", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        titles(&json_data(&output)),
        ["Blocked low", "Progress critical", "Todo critical"]
    );
}

#[test]
fn issue_list_orders_priority_before_creation_time() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");

    let low = create_issue(&app, "alpha", "Older low", None, Some("low"));
    let critical = create_issue(&app, "alpha", "Newer critical", None, Some("critical"));
    set_issue_fields(
        &app,
        &low,
        "todo",
        None,
        "2026-08-08T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &critical,
        "todo",
        None,
        "2026-08-10T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );

    let output = app
        .command()
        .args(["issue", "list", "--all-projects", "--json"])
        .output()
        .unwrap();
    assert_eq!(titles(&json_data(&output)), ["Newer critical", "Older low"]);
}

#[test]
fn issue_list_orders_creation_time_before_stable_ties() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");
    create_project(&app, "beta");

    let newer_alpha = create_issue(&app, "alpha", "Newer alpha", None, Some("high"));
    let older_beta = create_issue(&app, "beta", "Older beta", None, Some("high"));
    set_issue_fields(
        &app,
        &newer_alpha,
        "todo",
        None,
        "2026-08-10T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );
    set_issue_fields(
        &app,
        &older_beta,
        "todo",
        None,
        "2026-08-08T00:00:00Z",
        "2026-08-15T00:00:00Z",
    );

    let output = app
        .command()
        .args(["issue", "list", "--all-projects", "--json"])
        .output()
        .unwrap();
    assert_eq!(titles(&json_data(&output)), ["Older beta", "Newer alpha"]);
}

#[test]
fn issue_list_uses_project_and_number_as_stable_ties() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "beta");
    create_project(&app, "alpha");

    let beta = create_issue(&app, "beta", "Beta one", None, Some("high"));
    let alpha_one = create_issue(&app, "alpha", "Alpha one", None, Some("high"));
    let alpha_two = create_issue(&app, "alpha", "Alpha two", None, Some("high"));
    for issue in [&beta, &alpha_one, &alpha_two] {
        set_issue_fields(
            &app,
            issue,
            "todo",
            None,
            "2026-08-10T00:00:00Z",
            "2026-08-15T00:00:00Z",
        );
    }

    let output = app
        .command()
        .args(["issue", "list", "--all-projects", "--json"])
        .output()
        .unwrap();
    let data = json_data(&output);
    assert_eq!(titles(&data), ["Alpha one", "Alpha two", "Beta one"]);
    assert_eq!(data[0]["number"], 1);
    assert_eq!(data[1]["number"], 2);
    assert_eq!(data[2]["number"], 1);
}
