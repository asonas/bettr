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
fn project_list_requires_an_initialized_database() {
    let app = crate::support::TestApp::new();

    app.command()
        .args(["project", "list", "--json"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("database_not_initialized"));
}
