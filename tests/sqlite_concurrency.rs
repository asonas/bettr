mod support;

const LOCK_HELPER_DATABASE: &str = "BETTR_TEST_LOCK_DATABASE";
const LOCK_ACQUIRED: &str = "BETTR_TEST_LOCK_ACQUIRED";
const RELEASE_LOCK: &str = "BETTR_TEST_RELEASE_LOCK";

struct LockedWriter {
    child: Option<std::process::Child>,
    stdout: Option<std::io::BufReader<std::process::ChildStdout>>,
}

impl LockedWriter {
    fn start(database: &std::path::Path) -> Self {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hold_immediate_writer_for_parent_process",
                "--nocapture",
            ])
            .env(LOCK_HELPER_DATABASE, database)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(
                std::io::BufRead::read_line(&mut stdout, &mut line).unwrap(),
                0,
                "lock helper exited before acquiring the writer lock"
            );
            if line.trim() == LOCK_ACQUIRED {
                break;
            }
        }
        Self {
            child: Some(child),
            stdout: Some(stdout),
        }
    }

    fn release(mut self) {
        assert!(self.release_and_wait().unwrap().unwrap().success());
    }

    fn release_and_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let Some(mut child) = self.child.take() else {
            return Ok(None);
        };
        let mut stdin = child.stdin.take().unwrap();
        let release_result =
            std::io::Write::write_all(&mut stdin, format!("{RELEASE_LOCK}\n").as_bytes());
        drop(stdin);
        let status = child.wait();
        drop(self.stdout.take());
        release_result?;
        status.map(Some)
    }
}

impl Drop for LockedWriter {
    fn drop(&mut self) {
        let _ = self.release_and_wait();
    }
}

fn bettr_command(database: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_bettr"));
    command.arg("--database").arg(database).env_clear();
    command
}

fn initialized_project() -> crate::support::TestApp {
    let app = crate::support::TestApp::new();
    app.command().arg("init").assert().success();
    app.command()
        .args(["project", "create", "bettr"])
        .assert()
        .success();
    app
}

fn output_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

#[test]
fn hold_immediate_writer_for_parent_process() {
    let Some(database) = std::env::var_os(LOCK_HELPER_DATABASE) else {
        return;
    };
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute_batch("PRAGMA journal_mode = WAL; BEGIN IMMEDIATE;")
        .unwrap();
    println!("{LOCK_ACQUIRED}");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();

    let mut release = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut release).unwrap();
    assert_eq!(release.trim(), RELEASE_LOCK);
    connection.execute_batch("COMMIT;").unwrap();
}

#[test]
fn a_writer_succeeds_when_an_immediate_transaction_releases_before_the_timeout() {
    let app = initialized_project();
    let writer = LockedWriter::start(&app.database);
    let mut command = bettr_command(&app.database);
    command
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Created after lock",
            "--json",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = command.spawn().unwrap();

    writer.release();
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output_json(&output.stdout)["data"]["title"],
        "Created after lock"
    );
}

#[test]
fn a_writer_reports_database_busy_when_the_lock_outlives_the_timeout() {
    let app = initialized_project();
    let writer = LockedWriter::start(&app.database);

    let started_at = std::time::Instant::now();
    let output = bettr_command(&app.database)
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Times out",
            "--json",
        ])
        .output()
        .unwrap();
    let elapsed = started_at.elapsed();

    assert_eq!(output.status.code(), Some(5));
    assert_eq!(
        output_json(&output.stderr)["error"]["code"],
        "database_busy"
    );
    assert!(
        elapsed >= std::time::Duration::from_secs(5),
        "writer returned before the configured busy timeout: {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(12),
        "writer retried beyond a single busy timeout: {elapsed:?}"
    );
    writer.release();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM issues WHERE title = 'Times out'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn wal_readers_continue_while_an_immediate_writer_is_open() {
    let app = initialized_project();
    let writer = LockedWriter::start(&app.database);

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM projects", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );

    writer.release();
}

#[test]
fn concurrent_issue_and_comment_processes_preserve_database_integrity() {
    const ISSUE_COUNT: i64 = 8;

    let app = initialized_project();
    let mut issue_processes = Vec::new();
    for index in 1..=ISSUE_COUNT {
        let mut command = bettr_command(&app.database);
        command
            .args([
                "issue",
                "create",
                "--project",
                "bettr",
                "--title",
                &format!("Concurrent issue {index}"),
                "--json",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        issue_processes.push(command.spawn().unwrap());
    }

    let mut issue_numbers = Vec::new();
    for child in issue_processes {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "issue creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        issue_numbers.push(
            output_json(&output.stdout)["data"]["number"]
                .as_i64()
                .unwrap(),
        );
    }
    issue_numbers.sort_unstable();
    assert_eq!(issue_numbers, (1..=ISSUE_COUNT).collect::<Vec<_>>());

    let mut comment_processes = Vec::new();
    for number in &issue_numbers {
        let mut command = bettr_command(&app.database);
        command
            .args([
                "issue",
                "comment",
                &number.to_string(),
                "--project",
                "bettr",
                "--body",
                &format!("Concurrent comment for issue {number}"),
                "--json",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        comment_processes.push(command.spawn().unwrap());
    }
    for child in comment_processes {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "comment creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let foreign_key_violations = connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(foreign_key_violations.is_empty());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM comments", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        ISSUE_COUNT
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2 + (2 * ISSUE_COUNT)
    );
    for (operation, expected) in [
        ("init", 1),
        ("project_create", 1),
        ("issue_create", ISSUE_COUNT),
        ("issue_comment", ISSUE_COUNT),
    ] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM audit_events
                     WHERE operation = ?1 AND success = 1",
                    [operation],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            expected,
            "unexpected successful audit count for {operation}"
        );
    }
}
