mod support;

use predicates::prelude::*;
use std::os::unix::fs::MetadataExt;

fn sqlite_sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

fn directory_entries(path: &std::path::Path) -> Vec<std::ffi::OsString> {
    let mut entries = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn assert_project_list_rejected_without_file_changes(
    app: &crate::support::TestApp,
    expected_bytes: &[u8],
    private_contents: &str,
) {
    let expected_directory_entries = directory_entries(app.dir.path());
    let output = app
        .command()
        .args(["project", "list", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("database_not_initialized"));
    assert!(!stderr.contains(private_contents));
    assert_eq!(std::fs::read(&app.database).unwrap(), expected_bytes);
    assert_eq!(
        directory_entries(app.dir.path()),
        expected_directory_entries
    );
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(!sqlite_sidecar(&app.database, suffix).exists());
    }
}

#[test]
fn init_creates_a_version_four_database_with_migration_history() {
    let app = crate::support::TestApp::new();

    app.command()
        .args(["init", "--json"])
        .assert()
        .success()
        .stdout("{\"schema_version\":1,\"data\":{\"initialized\":true}}\n");

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    let migrations = connection
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        migrations,
        vec![
            (1, "phase1_baseline".to_owned()),
            (2, "schema_migrations".to_owned()),
            (3, "phase_two_coordination".to_owned()),
            (4, "idempotency_and_audit".to_owned()),
        ]
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE operation = 'init' AND success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    drop(connection);
    let inode = std::fs::metadata(&app.database).unwrap().ino();

    app.command()
        .args(["init", "--json"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("database_already_initialized"));

    assert!(app.database.is_file());
    assert_eq!(std::fs::metadata(&app.database).unwrap().ino(), inode);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                 WHERE operation = 'init' AND success = 0 AND exit_code = 2",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn init_replays_success_for_the_same_idempotency_key() {
    let app = crate::support::TestApp::new();
    let arguments = ["init", "--idempotency-key", "init-1", "--json"];
    app.command().args(arguments).assert().success();
    app.command().args(arguments).assert().success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE operation = 'init' AND success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM idempotency_records WHERE idempotency_key = 'init-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn project_list_migrates_a_version_one_database_and_records_history() {
    let app = crate::support::TestApp::new();

    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS schema_migrations;\n\
             PRAGMA user_version = 1;",
        )
        .unwrap();
    drop(connection);

    app.command()
        .args(["project", "list", "--json"])
        .assert()
        .success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE name = 'bettr'",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
        1
    );
    let migrations = connection
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        migrations,
        vec![
            (1, "phase1_baseline".to_owned()),
            (2, "schema_migrations".to_owned()),
            (3, "phase_two_coordination".to_owned()),
            (4, "idempotency_and_audit".to_owned()),
        ]
    );
    let audit_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE operation = 'schema_migrate' AND success = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count, 3);
    let metadata: String = connection
        .query_row(
            "SELECT metadata_json FROM audit_events
             WHERE operation = 'schema_migrate' AND success = 1
             ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&metadata).unwrap(),
        serde_json::json!({
            "from_version": 3,
            "to_version": 4,
            "migration": "idempotency_and_audit",
        })
    );
}

#[test]
fn project_list_rejects_an_unknown_bettr_schema_version_without_changes() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 99;")
        .unwrap();
    drop(connection);
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = sqlite_sidecar(&app.database, suffix);
        match std::fs::remove_file(sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove SQLite sidecar: {error}"),
        }
    }
    let expected_bytes = std::fs::read(&app.database).unwrap();
    let expected_directory_entries = directory_entries(app.dir.path());

    let output = app
        .command()
        .args(["project", "list", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(
        response["error"]["code"],
        "unsupported_database_schema_version"
    );
    assert_eq!(response["error"]["details"]["found_version"], 99);
    assert_eq!(response["error"]["details"]["current_version"], 4);
    assert_eq!(std::fs::read(&app.database).unwrap(), expected_bytes);
    assert_eq!(
        directory_entries(app.dir.path()),
        expected_directory_entries
    );
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(!sqlite_sidecar(&app.database, suffix).exists());
    }
}

#[test]
fn init_does_not_modify_a_non_bettr_database_with_similar_table_names() {
    let app = crate::support::TestApp::new();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE projects (sentinel TEXT);
             CREATE TABLE issues (sentinel TEXT);
             CREATE TABLE comments (sentinel TEXT);
             CREATE TABLE domain_events (sentinel TEXT);
             CREATE TABLE audit_events (
                 c01 TEXT, c02 TEXT, c03 TEXT, c04 TEXT,
                 c05 TEXT, c06 TEXT, c07 TEXT, c08 TEXT,
                 c09 TEXT, c10 TEXT, c11 TEXT, c12 TEXT,
                 c13 TEXT, c14 TEXT, c15 TEXT, c16 TEXT
             );
             INSERT INTO projects VALUES ('keep-me');
             PRAGMA user_version = 1;",
        )
        .unwrap();
    let journal_mode = connection
        .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
        .unwrap();
    let schema = connection
        .prepare("SELECT name, sql FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(connection);
    let bytes = std::fs::read(&app.database).unwrap();

    app.command()
        .args(["init", "--json"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("database_already_initialized"));

    assert_eq!(std::fs::read(&app.database).unwrap(), bytes);
    let connection = rusqlite::Connection::open_with_flags(
        &app.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .unwrap(),
        journal_mode
    );
    let after_schema = connection
        .prepare("SELECT name, sql FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(after_schema, schema);
    assert_eq!(
        connection
            .query_row("SELECT sentinel FROM projects", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "keep-me"
    );
}

#[test]
fn project_list_rejects_a_database_with_the_wrong_application_id_without_side_effects() {
    let app = crate::support::TestApp::new();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE projects (sentinel TEXT);\n\
             INSERT INTO projects VALUES ('keep-me');\n\
             PRAGMA application_id = 305419896;\n\
             PRAGMA user_version = 1;",
        )
        .unwrap();
    let journal_mode = connection
        .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
        .unwrap();
    let schema = connection
        .prepare("SELECT name, sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let data = connection
        .query_row("SELECT sentinel FROM projects", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    drop(connection);
    let bytes = std::fs::read(&app.database).unwrap();

    assert_project_list_rejected_without_file_changes(&app, &bytes, "keep-me");

    let connection = rusqlite::Connection::open_with_flags(
        &app.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .unwrap(),
        journal_mode
    );
    let after_schema = connection
        .prepare("SELECT name, sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(after_schema, schema);
    assert_eq!(
        connection
            .query_row("SELECT sentinel FROM projects", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        data
    );
}

#[test]
fn project_list_does_not_touch_a_non_bettr_wal_database() {
    let app = crate::support::TestApp::new();
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE sentinel (value TEXT NOT NULL);
             INSERT INTO sentinel VALUES ('keep-me');
             PRAGMA wal_checkpoint(TRUNCATE);",
        )
        .unwrap();
    let journal_mode = connection
        .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
        .unwrap();
    let schema = connection
        .prepare("SELECT name, sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let data = connection
        .query_row("SELECT value FROM sentinel", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    drop(connection);

    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = sqlite_sidecar(&app.database, suffix);
        match std::fs::remove_file(sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove SQLite sidecar: {error}"),
        }
    }
    let bytes = std::fs::read(&app.database).unwrap();

    assert_project_list_rejected_without_file_changes(&app, &bytes, "keep-me");

    let connection = rusqlite::Connection::open_with_flags(
        &app.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .unwrap(),
        journal_mode
    );
    let after_schema = connection
        .prepare("SELECT name, sql FROM sqlite_schema ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(after_schema, schema);
    assert_eq!(
        connection
            .query_row("SELECT value FROM sentinel", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        data
    );
}

#[test]
fn project_list_rejects_a_short_database_file_without_side_effects() {
    let app = crate::support::TestApp::new();
    let bytes = b"SQLite format 3\0private short file";
    std::fs::write(&app.database, bytes).unwrap();

    assert_project_list_rejected_without_file_changes(&app, bytes, "private short file");
}

#[test]
fn project_list_rejects_a_non_sqlite_file_without_side_effects() {
    let app = crate::support::TestApp::new();
    let mut bytes = vec![0_u8; 100];
    bytes[..25].copy_from_slice(b"private database contents");
    std::fs::write(&app.database, &bytes).unwrap();

    assert_project_list_rejected_without_file_changes(&app, &bytes, "private database contents");
}

#[cfg(unix)]
#[test]
fn project_list_rejects_a_fifo_without_waiting_for_a_writer() {
    let app = crate::support::TestApp::new();
    let status = std::process::Command::new("mkfifo")
        .arg(&app.database)
        .status()
        .unwrap();
    assert!(status.success());

    let output = app
        .command()
        .args(["project", "list", "--json"])
        .timeout(std::time::Duration::from_secs(1))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(response["error"]["code"], "database_not_initialized");
}

#[cfg(unix)]
#[test]
fn project_list_accepts_a_symlink_to_a_valid_bettr_database() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    let target = app.dir.path().join("target.db");
    std::fs::rename(&app.database, &target).unwrap();
    std::os::unix::fs::symlink(&target, &app.database).unwrap();

    app.command()
        .args(["project", "list", "--json"])
        .assert()
        .success();
}

#[test]
fn project_list_accepts_a_valid_bettr_database() {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();

    let output = app
        .command()
        .args(["project", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], 1);
    assert_eq!(response["data"], serde_json::json!([]));
}

#[test]
fn project_list_requires_an_initialized_database() {
    let app = crate::support::TestApp::new();

    app.command()
        .args(["project", "list", "--json"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("database_not_initialized"));
}

#[cfg(unix)]
#[test]
fn init_creates_the_direct_database_parent_with_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("private/bettr.db");
    let mut command = assert_cmd::Command::cargo_bin("bettr").unwrap();
    command
        .env_clear()
        .env("HOME", directory.path().join("home"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .arg("--database")
        .arg(&database)
        .arg("init")
        .assert()
        .success();

    let mode = std::fs::metadata(database.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}
