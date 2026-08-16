mod support;

fn create_project(app: &crate::support::TestApp, name: &str) {
    app.command()
        .args(["project", "create", name])
        .assert()
        .success();
}

fn create_issue(app: &crate::support::TestApp, project: &str, title: &str) {
    app.command()
        .args(["issue", "create", "--project", project, "--title", title])
        .assert()
        .success();
}

fn json_data(output: &std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

fn initialized_app() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    create_project(&app, "alpha");
    create_project(&app, "beta");
    create_issue(&app, "alpha", "Build the API");
    create_issue(&app, "beta", "Consume the API");
    create_issue(&app, "beta", "Document the API");
    app
}

#[test]
fn dependency_commands_add_list_and_reject_cycles() {
    let app = initialized_app();

    let added = app
        .command()
        .args(["issue", "dependency", "add", "alpha#1", "beta#1", "--json"])
        .output()
        .unwrap();
    let relation = json_data(&added);
    assert_eq!(relation["relation"], "blocks");
    assert_eq!(relation["blocker"], "alpha#1");
    assert_eq!(relation["blocked"], "beta#1");

    let listed = app
        .command()
        .args([
            "issue",
            "dependency",
            "list",
            "1",
            "--project",
            "beta",
            "--json",
        ])
        .output()
        .unwrap();
    let relations = json_data(&listed);
    assert_eq!(relations.as_array().unwrap().len(), 1);
    assert_eq!(relations[0]["blocker"], "alpha#1");

    app.command()
        .args(["issue", "dependency", "add", "alpha#1", "beta#1", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));

    app.command()
        .args(["issue", "dependency", "add", "beta#1", "alpha#1", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));
}

#[test]
fn parent_commands_enforce_one_level_and_one_parent() {
    let app = initialized_app();

    app.command()
        .args(["issue", "parent", "set", "beta#1", "alpha#1", "--json"])
        .assert()
        .success();

    let listed = app
        .command()
        .args([
            "issue",
            "parent",
            "list",
            "1",
            "--project",
            "beta",
            "--json",
        ])
        .output()
        .unwrap();
    let parents = json_data(&listed);
    assert_eq!(parents.as_array().unwrap().len(), 1);
    assert_eq!(parents[0]["child"], "beta#1");
    assert_eq!(parents[0]["parent"], "alpha#1");

    app.command()
        .args(["issue", "parent", "set", "alpha#1", "beta#2", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));

    app.command()
        .args(["issue", "parent", "set", "beta#2", "beta#1", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));

    app.command()
        .args(["issue", "parent", "set", "beta#1", "alpha#1", "--json"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("conflict"));
}
