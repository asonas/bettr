mod support;

use predicates::prelude::*;
use std::os::unix::fs::MetadataExt;

#[test]
fn init_creates_a_version_one_database_once() {
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
        1
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
fn project_list_requires_an_initialized_database() {
    let app = crate::support::TestApp::new();

    app.command()
        .args(["project", "list", "--json"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("database_not_initialized"));
}
