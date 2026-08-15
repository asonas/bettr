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

#[test]
fn issue_list_requires_a_project_by_default_and_excludes_completed_issues() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");
    create_project(&app, "beta");

    let todo = create_issue(&app, "alpha", "Alpha todo", None, Some("low"));
    let done = create_issue(&app, "alpha", "Alpha done", None, Some("high"));
    let cancelled = create_issue(&app, "alpha", "Alpha cancelled", None, None);
    create_issue(&app, "beta", "Beta todo", None, Some("urgent"));
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
        Some("urgent"),
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

    let priorities = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--priority",
            "high",
            "--priority",
            "urgent",
            "--json",
        ])
        .output()
        .unwrap();
    let priorities = json_data(&priorities);
    assert_eq!(priorities.as_array().unwrap().len(), 3);

    let assignee_and_time = app
        .command()
        .args([
            "issue",
            "list",
            "--all-projects",
            "--assignee",
            "alice",
            "--updated-after",
            "2026-08-14T00:00:00Z",
            "--json",
        ])
        .output()
        .unwrap();
    let assignee_and_time = json_data(&assignee_and_time);
    assert_eq!(assignee_and_time.as_array().unwrap().len(), 1);
    assert_eq!(assignee_and_time[0]["title"], "Blocked work");

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
