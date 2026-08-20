mod support;

fn json_data(output: &std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

fn init_git_worktree(path: &std::path::Path, branch: &str) -> String {
    std::fs::create_dir_all(path).unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "-b", branch])
        .current_dir(path)
        .status()
        .unwrap();
    assert!(status.success(), "git init failed for {}", path.display());
    path.canonicalize().unwrap().to_str().unwrap().to_owned()
}

fn initialized_app() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "alpha"])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "alpha",
            "--title",
            "Track the current worktree",
        ])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "alpha",
            "--title",
            "Use the same worktree",
        ])
        .assert()
        .success();
    app
}

#[test]
fn issue_worktrees_start_empty_attach_current_and_keep_history() {
    let app = initialized_app();

    let created = app
        .command()
        .args(["issue", "show", "1", "--project", "alpha", "--json"])
        .output()
        .unwrap();
    let created = json_data(&created);
    assert_eq!(created["worktrees"].as_array().unwrap().len(), 0);

    let current_path = app.dir.path().join("work");
    let current_path = init_git_worktree(&current_path, "feature/main");
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "alpha",
            "--title",
            "Claim from the same worktree",
        ])
        .assert()
        .success();
    app.command()
        .env("BETTR_AGENT", "codex")
        .env("BETTR_SESSION_ID", "worktree-session")
        .args(["issue", "claim", "3", "--project", "alpha", "--json"])
        .assert()
        .success();
    let claimed = app
        .command()
        .args(["issue", "show", "3", "--project", "alpha", "--json"])
        .output()
        .unwrap();
    let claimed = json_data(&claimed);
    assert_eq!(claimed["worktrees"][0]["path"], current_path);
    assert_eq!(claimed["worktrees"][0]["active"], true);

    app.command()
        .args([
            "issue",
            "start",
            "1",
            "--project",
            "alpha",
            "--revision",
            "1",
            "--json",
        ])
        .assert()
        .success();

    let shown = app
        .command()
        .args(["issue", "show", "1", "--project", "alpha", "--json"])
        .output()
        .unwrap();
    let shown = json_data(&shown);
    assert_eq!(shown["worktrees"].as_array().unwrap().len(), 1);
    assert_eq!(shown["worktrees"][0]["path"], current_path);
    assert_eq!(shown["worktrees"][0]["branch"], "feature/main");
    assert_eq!(shown["worktrees"][0]["active"], true);

    app.command()
        .args([
            "issue",
            "start",
            "2",
            "--project",
            "alpha",
            "--revision",
            "1",
            "--json",
        ])
        .assert()
        .success();

    let second_path = app.dir.path().join("other");
    let second_path = init_git_worktree(&second_path, "feature/other");
    let mut add = app.command();
    add.args(["issue", "worktree", "add", "alpha#1", "--path"])
        .arg(&second_path)
        .arg("--json")
        .assert()
        .success();

    let shown = app
        .command()
        .args(["issue", "show", "1", "--project", "alpha", "--json"])
        .output()
        .unwrap();
    let shown = json_data(&shown);
    let worktrees = shown["worktrees"].as_array().unwrap();
    assert_eq!(worktrees.len(), 2);
    assert!(
        worktrees
            .iter()
            .any(|worktree| { worktree["path"] == current_path && worktree["active"] == true })
    );
    assert!(worktrees.iter().any(|worktree| {
        worktree["path"] == second_path && worktree["branch"] == "feature/other"
    }));

    let mut remove = app.command();
    remove
        .args(["issue", "worktree", "remove", "alpha#1", "--path"])
        .arg(&current_path)
        .arg("--json")
        .assert()
        .success();

    let listed = app
        .command()
        .args(["issue", "worktree", "list", "alpha#1", "--json"])
        .output()
        .unwrap();
    let listed = json_data(&listed);
    let worktrees = listed.as_array().unwrap();
    assert_eq!(worktrees.len(), 2);
    assert!(
        worktrees
            .iter()
            .any(|worktree| { worktree["path"] == current_path && worktree["active"] == false })
    );
    assert!(
        worktrees
            .iter()
            .any(|worktree| { worktree["path"] == second_path && worktree["active"] == true })
    );

    app.command()
        .args([
            "issue",
            "complete",
            "1",
            "--project",
            "alpha",
            "--revision",
            "2",
            "--summary",
            "Worktree tracking is complete",
            "--verification",
            "CLI test passed",
            "--json",
        ])
        .assert()
        .success();

    let shown = app
        .command()
        .args(["issue", "show", "1", "--project", "alpha", "--json"])
        .output()
        .unwrap();
    let shown = json_data(&shown);
    assert!(
        shown["worktrees"]
            .as_array()
            .unwrap()
            .iter()
            .all(|worktree| worktree["active"] == false)
    );
}
