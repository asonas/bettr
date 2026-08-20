mod support;

fn sqlite_sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

fn command_for(app: &crate::support::TestApp, database: &std::path::Path) -> assert_cmd::Command {
    let mut command = assert_cmd::Command::cargo_bin("bettr").unwrap();
    command
        .arg("--database")
        .arg(database)
        .env_clear()
        .current_dir(app.dir.path().join("work"))
        .env("HOME", app.dir.path().join("home"))
        .env("XDG_CONFIG_HOME", app.dir.path().join("config"))
        .env("XDG_DATA_HOME", app.dir.path().join("data"));
    command
}

#[test]
fn backup_writes_a_single_sqlite_snapshot_to_the_explicit_output() {
    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();
    app.command()
        .args(["project", "create", "bettr", "--json"])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "snapshot issue",
            "--body",
            "snapshot body",
            "--json",
        ])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "snapshot comment",
            "--json",
        ])
        .assert()
        .success();

    let backup = app.dir.path().join("snapshot.db");
    let backup_result = app
        .command()
        .args(["backup", "--output", backup.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(backup_result.status.success());
    let backup_response: serde_json::Value = serde_json::from_slice(&backup_result.stdout).unwrap();
    assert_eq!(backup_response["data"]["format"], "sqlite_online_backup");
    assert_eq!(backup_response["data"]["schema_version"], 8);

    assert!(backup.is_file());
    let snapshot_bytes = std::fs::read(&backup).unwrap();
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = backup.as_os_str().to_owned();
        sidecar.push(suffix);
        assert!(!std::path::PathBuf::from(sidecar).exists());
    }

    let second_backup = app
        .command()
        .args(["backup", "--output", backup.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert_eq!(second_backup.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&second_backup.stderr).contains("backup_output_exists"));
    assert_eq!(std::fs::read(&backup).unwrap(), snapshot_bytes);

    let audit = std::fs::read_to_string(app.database.with_extension("audit.jsonl")).unwrap();
    assert!(audit.contains("\"operation\":\"backup\""));
    assert!(!audit.contains(backup.to_str().unwrap()));
    assert!(!audit.contains("snapshot comment"));
}

#[test]
fn backup_captures_committed_wal_state_while_database_is_open() {
    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();
    app.command()
        .args(["project", "create", "bettr", "--json"])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "before wal update",
            "--json",
        ])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             UPDATE issues SET title = 'committed in wal' WHERE number = 1;
             BEGIN;",
        )
        .unwrap();
    let _: String = connection
        .query_row("SELECT title FROM issues WHERE number = 1", [], |row| {
            row.get(0)
        })
        .unwrap();

    let backup = app.dir.path().join("wal-snapshot.db");
    app.command()
        .args(["backup", "--output", backup.to_str().unwrap(), "--json"])
        .assert()
        .success();

    connection.execute_batch("ROLLBACK").unwrap();
    drop(connection);

    let restored = app.dir.path().join("wal-restored.db");
    app.command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--yes",
            "--json",
        ])
        .assert()
        .success();
    let output = command_for(&app, &restored)
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let issue: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(issue["data"]["title"], "committed in wal");
}

#[test]
fn restore_preserves_issue_history_revision_audit_and_jsonl() {
    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();
    app.command()
        .args(["project", "create", "bettr", "--json"])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "original title",
            "--body",
            "original body",
            "--json",
        ])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "edit",
            "1",
            "--project",
            "bettr",
            "--revision",
            "1",
            "--title",
            "revised title",
            "--json",
        ])
        .assert()
        .success();
    app.command()
        .args([
            "issue",
            "comment",
            "1",
            "--project",
            "bettr",
            "--body",
            "original comment",
            "--json",
        ])
        .assert()
        .success();

    let backup = app.dir.path().join("snapshot.db");
    app.command()
        .args(["backup", "--output", backup.to_str().unwrap(), "--json"])
        .assert()
        .success();

    let restored = app.dir.path().join("restored.db");
    app.command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--yes",
            "--json",
        ])
        .assert()
        .success();

    let issue_output = command_for(&app, &restored)
        .args(["issue", "show", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    assert!(issue_output.status.success());
    let issue: serde_json::Value = serde_json::from_slice(&issue_output.stdout).unwrap();
    assert_eq!(issue["data"]["title"], "revised title");
    assert_eq!(issue["data"]["body"], "original body");
    assert_eq!(issue["data"]["revision"], 2);

    let history_output = command_for(&app, &restored)
        .args(["issue", "history", "1", "--project", "bettr", "--json"])
        .output()
        .unwrap();
    assert!(history_output.status.success());
    let history: serde_json::Value = serde_json::from_slice(&history_output.stdout).unwrap();
    assert!(history["data"].as_array().unwrap().iter().any(|event| {
        event["event_type"] == "comment_added" && event["metadata"]["body"] == "original comment"
    }));

    let audit_output = command_for(&app, &restored)
        .args(["audit", "list", "--json"])
        .output()
        .unwrap();
    assert!(audit_output.status.success());
    let audit: serde_json::Value = serde_json::from_slice(&audit_output.stdout).unwrap();
    assert!(
        audit["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["operation"] == "restore")
    );

    command_for(&app, &restored)
        .args(["audit", "verify", "--json"])
        .assert()
        .success();
    assert!(restored.with_extension("audit.jsonl").is_file());
}

#[test]
fn restore_requires_confirmation_and_never_implicitly_replaces_output() {
    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();
    app.command()
        .args(["project", "create", "bettr", "--json"])
        .assert()
        .success();
    let backup = app.dir.path().join("snapshot.db");
    app.command()
        .args(["backup", "--output", backup.to_str().unwrap(), "--json"])
        .assert()
        .success();

    let restored = app.dir.path().join("restored.db");
    std::fs::write(&restored, b"keep this file").unwrap();

    let without_confirmation = app
        .command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(without_confirmation.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&without_confirmation.stderr).contains("confirmation_required")
    );
    assert_eq!(std::fs::read(&restored).unwrap(), b"keep this file");
    let source_audit = std::fs::read_to_string(app.database.with_extension("audit.jsonl")).unwrap();
    assert!(source_audit.contains("\"operation\":\"restore\""));
    assert!(!source_audit.contains(backup.to_str().unwrap()));

    let without_replace = app
        .command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(without_replace.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&without_replace.stderr).contains("backup_output_exists"));
    assert_eq!(std::fs::read(&restored).unwrap(), b"keep this file");

    std::fs::write(sqlite_sidecar(&restored, "-wal"), b"active sidecar").unwrap();
    let with_sidecar = app
        .command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--replace",
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(with_sidecar.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&with_sidecar.stderr).contains("backup_destination_in_use"));
    assert_eq!(std::fs::read(&restored).unwrap(), b"keep this file");
    std::fs::remove_file(sqlite_sidecar(&restored, "-wal")).unwrap();

    app.command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--replace",
            "--yes",
            "--json",
        ])
        .assert()
        .success();
    assert_ne!(std::fs::read(&restored).unwrap(), b"keep this file");
}

#[test]
fn restore_rejects_corrupt_and_unrelated_sqlite_inputs_without_creating_output() {
    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();

    let corrupt = app.dir.path().join("corrupt.db");
    std::fs::write(&corrupt, b"not a sqlite backup").unwrap();
    let corrupt_output = app.dir.path().join("corrupt-restored.db");
    let corrupt_result = app
        .command()
        .args([
            "restore",
            "--input",
            corrupt.to_str().unwrap(),
            "--output",
            corrupt_output.to_str().unwrap(),
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(corrupt_result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&corrupt_result.stderr).contains("invalid_backup"));
    assert!(!corrupt_output.exists());

    let unrelated = app.dir.path().join("unrelated.db");
    let connection = rusqlite::Connection::open(&unrelated).unwrap();
    connection
        .execute("CREATE TABLE unrelated (value TEXT)", [])
        .unwrap();
    drop(connection);
    let unrelated_output = app.dir.path().join("unrelated-restored.db");
    let unrelated_result = app
        .command()
        .args([
            "restore",
            "--input",
            unrelated.to_str().unwrap(),
            "--output",
            unrelated_output.to_str().unwrap(),
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(unrelated_result.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&unrelated_result.stderr).contains("database_not_initialized"));
    assert!(!unrelated_output.exists());
}

#[cfg(unix)]
#[test]
fn backup_reports_permission_errors_without_leaking_filesystem_details() {
    use std::os::unix::fs::PermissionsExt as _;

    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();
    let protected = app.dir.path().join("protected");
    std::fs::create_dir(&protected).unwrap();
    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o500)).unwrap();

    let output = protected.join("snapshot.db");
    let result = app
        .command()
        .args(["backup", "--output", output.to_str().unwrap(), "--json"])
        .output()
        .unwrap();

    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result.status.code(), Some(10));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("backup_operation_failed"));
    assert!(!stderr.contains("Operation not permitted"));
    assert!(!stderr.contains(output.to_str().unwrap()));
    assert!(!output.exists());
}

#[test]
fn restore_rejects_future_schema_versions() {
    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();
    let backup = app.dir.path().join("snapshot.db");
    app.command()
        .args(["backup", "--output", backup.to_str().unwrap(), "--json"])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&backup).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);

    let restored = app.dir.path().join("restored.db");
    let result = app
        .command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("unsupported_database_schema_version"));
    assert!(!restored.exists());
}

#[test]
fn restore_rejects_a_backup_with_sqlite_sidecars() {
    let app = crate::support::TestApp::new();
    app.command().args(["init", "--json"]).assert().success();
    let backup = app.dir.path().join("snapshot.db");
    app.command()
        .args(["backup", "--output", backup.to_str().unwrap(), "--json"])
        .assert()
        .success();
    std::fs::write(sqlite_sidecar(&backup, "-wal"), b"stale sidecar").unwrap();

    let restored = app.dir.path().join("restored.db");
    let result = app
        .command()
        .args([
            "restore",
            "--input",
            backup.to_str().unwrap(),
            "--output",
            restored.to_str().unwrap(),
            "--yes",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid_backup"));
    assert!(!restored.exists());
}
