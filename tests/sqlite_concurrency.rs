#[allow(dead_code)]
#[path = "../src/app.rs"]
mod app;
#[allow(dead_code)]
#[path = "../src/domain.rs"]
mod domain;
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;
#[allow(dead_code)]
#[path = "../src/store/mod.rs"]
mod store;
mod support;

const LOCK_HELPER_DATABASE: &str = "BETTR_TEST_LOCK_DATABASE";
const LOCK_ACQUIRED: &str = "BETTR_TEST_LOCK_ACQUIRED";
const RELEASE_LOCK: &str = "BETTR_TEST_RELEASE_LOCK";
const CONTENDER_DATABASE: &str = "BETTR_TEST_CONTENDER_DATABASE";
const CONTENDER_REACHED_BUSY_HANDLER: &str = "BETTR_TEST_CONTENDER_REACHED_BUSY_HANDLER";
const GATED_DATABASE: &str = "BETTR_TEST_GATED_DATABASE";
const GATED_ARGUMENTS: &str = "BETTR_TEST_GATED_ARGUMENTS";
const GATED_AGENT: &str = "BETTR_TEST_GATED_AGENT";
const GATED_SESSION_ID: &str = "BETTR_TEST_GATED_SESSION_ID";
const GATE_READY: &str = "BETTR_TEST_GATE_READY";
const OPEN_GATE: u8 = b'G';

static CONTENDER_SIGNAL_SENT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn signal_contender_reached_busy_handler(_attempt: i32) -> bool {
    if !CONTENDER_SIGNAL_SENT.swap(true, std::sync::atomic::Ordering::Relaxed) {
        use std::io::Write as _;

        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{CONTENDER_REACHED_BUSY_HANDLER}");
        let _ = stdout.flush();
    }
    true
}

struct TestProcess {
    child: Option<std::process::Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::io::BufReader<std::process::ChildStdout>>,
}

impl TestProcess {
    fn start(
        mut command: std::process::Command,
        stdin: std::process::Stdio,
        expected_signal: &str,
        premature_exit_message: &str,
    ) -> Self {
        let mut child = command
            .stdin(stdin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(
                std::io::BufRead::read_line(&mut stdout, &mut line).unwrap(),
                0,
                "{premature_exit_message}"
            );
            if line.trim() == expected_signal {
                break;
            }
        }
        Self {
            child: Some(child),
            stdin,
            stdout: Some(stdout),
        }
    }

    fn send_signal(&mut self, signal: &str) {
        let mut stdin = self.stdin.take().unwrap();
        std::io::Write::write_all(&mut stdin, format!("{signal}\n").as_bytes()).unwrap();
    }

    fn wait(mut self) {
        let mut child = self.child.take().unwrap();
        let status = child.wait().unwrap();
        drop(self.stdout.take());
        assert!(status.success());
    }

    fn output(mut self) -> std::process::Output {
        let mut child = self.child.take().unwrap();
        drop(self.stdin.take());
        let mut stdout = Vec::new();
        std::io::Read::read_to_end(&mut self.stdout.take().unwrap(), &mut stdout).unwrap();
        let status = child.wait().unwrap();
        std::process::Output {
            status,
            stdout,
            stderr: Vec::new(),
        }
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn helper_command(test_name: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", test_name, "--nocapture"]);
    command
}

struct LockedWriter(TestProcess);

impl LockedWriter {
    fn start(database: &std::path::Path) -> Self {
        let mut command = helper_command("hold_immediate_writer_for_parent_process");
        command.env(LOCK_HELPER_DATABASE, database);
        Self(TestProcess::start(
            command,
            std::process::Stdio::piped(),
            LOCK_ACQUIRED,
            "lock helper exited before acquiring the writer lock",
        ))
    }

    fn release(mut self) {
        self.0.send_signal(RELEASE_LOCK);
        self.0.wait();
    }
}

struct ContendingWriter(TestProcess);

impl ContendingWriter {
    fn start(database: &std::path::Path) -> Self {
        let mut command = helper_command("create_issue_after_busy_signal_for_parent_process");
        command.env(CONTENDER_DATABASE, database);
        Self(TestProcess::start(
            command,
            std::process::Stdio::null(),
            CONTENDER_REACHED_BUSY_HANDLER,
            "contender exited before reaching SQLite's busy handler",
        ))
    }

    fn wait(self) {
        self.0.wait();
    }
}

struct GatedBettr(TestProcess);

impl GatedBettr {
    fn start(database: &std::path::Path, arguments: &[String], gate: &StartGate) -> Self {
        Self::start_with_context(database, arguments, gate, None)
    }

    fn start_with_context(
        database: &std::path::Path,
        arguments: &[String],
        gate: &StartGate,
        context: Option<(&str, &str)>,
    ) -> Self {
        let mut command = helper_command("run_bettr_after_parent_opens_start_gate");
        command
            .env(GATED_DATABASE, database)
            .env(GATED_ARGUMENTS, serde_json::to_string(arguments).unwrap());
        if let Some((agent, session_id)) = context {
            command
                .env(GATED_AGENT, agent)
                .env(GATED_SESSION_ID, session_id);
        }
        Self(TestProcess::start(
            command,
            gate.child_stdin(),
            GATE_READY,
            "gated bettr process exited before reaching the start gate",
        ))
    }

    fn wait(self) {
        self.0.wait();
    }

    fn output(self) -> std::process::Output {
        self.0.output()
    }
}

struct StartGate {
    writer: std::os::unix::net::UnixStream,
    reader: std::os::unix::net::UnixStream,
}

impl StartGate {
    fn new() -> Self {
        let (writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
        Self { writer, reader }
    }

    fn child_stdin(&self) -> std::process::Stdio {
        let reader: std::os::fd::OwnedFd = self.reader.try_clone().unwrap().into();
        std::process::Stdio::from(reader)
    }

    fn open(mut self, process_count: usize) {
        use std::io::Write as _;

        drop(self.reader);
        self.writer
            .write_all(&vec![OPEN_GATE; process_count])
            .unwrap();
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

fn initialized_issue() -> crate::support::TestApp {
    let app = initialized_project();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Snapshot target",
        ])
        .assert()
        .success();
    app
}

fn downgrade_to_schema_version_one(app: &crate::support::TestApp) {
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS schema_migrations;\n\
             PRAGMA user_version = 1;",
        )
        .unwrap();
}

fn context() -> crate::domain::ExecutionContext {
    crate::domain::ExecutionContext {
        kind: crate::domain::InitiatorKind::Human,
        agent: None,
        session_id: None,
        operator: Some("snapshot-tester".to_owned()),
    }
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
fn create_issue_after_busy_signal_for_parent_process() {
    let Some(database_path) = std::env::var_os(CONTENDER_DATABASE) else {
        return;
    };
    let mut database = crate::store::Database::open(std::path::Path::new(&database_path)).unwrap();
    let busy_timeout = database
        .connection()
        .pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(busy_timeout, 5_000);
    database
        .connection()
        .busy_handler(Some(signal_contender_reached_busy_handler))
        .unwrap();

    let issue = database
        .create_issue(
            "bettr",
            &crate::domain::NewIssue {
                title: "Created after lock".to_owned(),
                body: None,
                priority: None,
            },
            &crate::domain::ExecutionContext {
                kind: crate::domain::InitiatorKind::System,
                agent: None,
                session_id: None,
                operator: None,
            },
        )
        .unwrap();
    assert_eq!(issue.title, "Created after lock");
}

#[test]
fn run_bettr_after_parent_opens_start_gate() {
    let Some(database_path) = std::env::var_os(GATED_DATABASE) else {
        return;
    };
    let arguments = serde_json::from_str::<Vec<String>>(
        &std::env::var(GATED_ARGUMENTS).expect("gated arguments"),
    )
    .unwrap();
    println!("{GATE_READY}");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();

    // SAFETY: this helper is always spawned with an open UnixStream as fd 0.
    let gate_fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(0) }
        .try_clone_to_owned()
        .unwrap();
    let mut gate_stream = std::os::unix::net::UnixStream::from(gate_fd);
    let mut gate_token = [0_u8];
    std::io::Read::read_exact(&mut gate_stream, &mut gate_token).unwrap();
    assert_eq!(gate_token[0], OPEN_GATE);

    let mut command = bettr_command(std::path::Path::new(&database_path));
    if let Some(agent) = std::env::var_os(GATED_AGENT) {
        command.env("BETTR_AGENT", agent);
    }
    if let Some(session_id) = std::env::var_os(GATED_SESSION_ID) {
        command.env("BETTR_SESSION_ID", session_id);
    }
    let output = command.args(arguments).output().unwrap();
    std::process::exit(output.status.code().unwrap_or(1));
}

#[test]
fn a_writer_succeeds_when_an_immediate_transaction_releases_before_the_timeout() {
    let app = initialized_project();
    let writer = LockedWriter::start(&app.database);
    let contender = ContendingWriter::start(&app.database);

    writer.release();
    contender.wait();

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM issues WHERE title = 'Created after lock'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
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
fn production_database_reads_continue_while_an_immediate_writer_is_open() {
    let app = initialized_project();
    let writer = LockedWriter::start(&app.database);

    let database = crate::store::Database::open(&app.database).unwrap();
    let projects = database.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "bettr");

    writer.release();
}

#[test]
fn production_database_connections_enable_integrity_and_contention_pragmas() {
    let app = initialized_project();
    let database = crate::store::Database::open(&app.database).unwrap();
    let connection = database.connection();

    assert_eq!(
        connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))
            .unwrap(),
        5_000
    );
}

#[test]
fn comment_lookup_and_audit_revision_share_the_immediate_transaction_snapshot() {
    let app = initialized_issue();
    let database = crate::store::Database::open(&app.database).unwrap();
    let (reached_tx, reached_rx) = std::sync::mpsc::sync_channel(0);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let mut paused = false;
    database
        .connection()
        .authorizer(Some(
            move |authorization: rusqlite::hooks::AuthContext<'_>| {
                if !paused
                    && matches!(
                        authorization.action,
                        rusqlite::hooks::AuthAction::Transaction {
                            operation: rusqlite::hooks::TransactionOperation::Begin
                        }
                    )
                {
                    paused = true;
                    reached_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
                rusqlite::hooks::Authorization::Allow
            },
        ))
        .unwrap();
    let mut app_under_test = crate::app::App::new(database);
    let operation = std::thread::spawn(move || {
        app_under_test.add_comment("bettr", 1, "Concurrent comment", &context())
    });

    reached_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let writer = rusqlite::Connection::open(&app.database).unwrap();
    writer
        .execute("UPDATE issues SET revision = 2 WHERE number = 1", [])
        .unwrap();
    release_tx.send(()).unwrap();
    operation.join().unwrap().unwrap();

    let audit_revision = writer
        .query_row(
            "SELECT revision FROM audit_events\n\
             WHERE operation = 'issue_comment' AND success = 1\n\
             ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(audit_revision, 2);
}

#[test]
fn history_events_and_audit_revision_share_one_read_snapshot() {
    let app = initialized_issue();
    let database = crate::store::Database::open(&app.database).unwrap();
    let (reached_tx, reached_rx) = std::sync::mpsc::sync_channel(0);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let mut paused = false;
    database
        .connection()
        .authorizer(Some(
            move |authorization: rusqlite::hooks::AuthContext<'_>| {
                if !paused
                    && matches!(
                        authorization.action,
                        rusqlite::hooks::AuthAction::Read {
                            table_name: "domain_events",
                            ..
                        }
                    )
                {
                    paused = true;
                    reached_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
                rusqlite::hooks::Authorization::Allow
            },
        ))
        .unwrap();
    let mut app_under_test = crate::app::App::new(database);
    let operation = std::thread::spawn(move || app_under_test.issue_history("bettr", 1));

    reached_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let writer = rusqlite::Connection::open(&app.database).unwrap();
    let (project_id, issue_id): (String, String) = writer
        .query_row(
            "SELECT project_id, id FROM issues WHERE number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let metadata = serde_json::json!({
        "changes": { "title": "Concurrent title" },
        "revision": 2,
        "context": context(),
    })
    .to_string();
    let transaction = writer.unchecked_transaction().unwrap();
    transaction
        .execute(
            "UPDATE issues SET title = 'Concurrent title', revision = 2 WHERE id = ?1",
            [&issue_id],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO domain_events (\n\
                 id, sequence, project_id, issue_id, event_type, metadata_json, created_at\n\
             ) VALUES (\n\
                 ?1, (SELECT COALESCE(MAX(sequence), 0) + 1 FROM domain_events),\n\
                 ?2, ?3, 'issue_updated', ?4, ?5\n\
             )",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                project_id,
                issue_id,
                metadata,
                chrono::Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();
    transaction.commit().unwrap();
    release_tx.send(()).unwrap();
    let history = operation.join().unwrap().unwrap();

    let max_history_revision = history.iter().filter_map(|event| event.revision).max();
    let audit_revision = writer
        .query_row(
            "SELECT revision FROM audit_events\n\
             WHERE operation = 'issue_history' AND success = 1\n\
             ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap();
    assert_eq!(audit_revision, max_history_revision);
}

#[test]
fn concurrent_issue_and_comment_processes_preserve_database_integrity() {
    const NEW_ISSUE_COUNT: i64 = 4;
    const COMMENT_COUNT: i64 = 4;

    let app = initialized_project();
    app.command()
        .args([
            "issue",
            "create",
            "--project",
            "bettr",
            "--title",
            "Comment target",
        ])
        .assert()
        .success();
    let gate = StartGate::new();
    let mut processes = Vec::new();
    for index in 1..=NEW_ISSUE_COUNT {
        processes.push(GatedBettr::start(
            &app.database,
            &[
                "issue".to_owned(),
                "create".to_owned(),
                "--project".to_owned(),
                "bettr".to_owned(),
                "--title".to_owned(),
                format!("Concurrent issue {index}"),
                "--json".to_owned(),
            ],
            &gate,
        ));
        processes.push(GatedBettr::start(
            &app.database,
            &[
                "issue".to_owned(),
                "comment".to_owned(),
                "1".to_owned(),
                "--project".to_owned(),
                "bettr".to_owned(),
                "--body".to_owned(),
                format!("Concurrent comment {index}"),
                "--json".to_owned(),
            ],
            &gate,
        ));
    }
    gate.open(processes.len());
    for process in processes {
        process.wait();
    }

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let issue_numbers = connection
        .prepare("SELECT number FROM issues ORDER BY number")
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        issue_numbers,
        (1..=(1 + NEW_ISSUE_COUNT)).collect::<Vec<_>>()
    );
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
        COMMENT_COUNT
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2 + 1 + NEW_ISSUE_COUNT + COMMENT_COUNT
    );
    for (operation, expected) in [
        ("init", 1),
        ("project_create", 1),
        ("issue_create", 1 + NEW_ISSUE_COUNT),
        ("issue_comment", COMMENT_COUNT),
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

#[test]
fn concurrent_claim_processes_allow_exactly_one_owner() {
    let app = initialized_issue();
    let arguments = vec![
        "issue".to_owned(),
        "claim".to_owned(),
        "1".to_owned(),
        "--project".to_owned(),
        "bettr".to_owned(),
        "--json".to_owned(),
    ];
    let gate = StartGate::new();
    let mut processes = Vec::new();
    for (agent, session_id) in [("codex", "session-a"), ("worker", "session-b")] {
        processes.push(GatedBettr::start_with_context(
            &app.database,
            &arguments,
            &gate,
            Some((agent, session_id)),
        ));
    }

    gate.open(processes.len());
    let outputs = processes
        .into_iter()
        .map(GatedBettr::output)
        .collect::<Vec<_>>();
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1
    );

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    let (state, assignee, revision, session_id): (String, Option<String>, i64, String) = connection
        .query_row(
            "SELECT issue.state, issue.assignee_name, issue.revision, lease.session_id
             FROM issues issue
             JOIN issue_leases lease ON lease.issue_id = issue.id
             WHERE issue.number = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(state, "in_progress");
    assert!(matches!(
        assignee.as_deref(),
        Some("codex") | Some("worker")
    ));
    assert_eq!(revision, 2);
    assert!(matches!(session_id.as_str(), "session-a" | "session-b"));
}

#[test]
fn concurrent_database_opens_apply_schema_migration_once() {
    let app = initialized_project();
    downgrade_to_schema_version_one(&app);

    let gate = StartGate::new();
    let mut processes = Vec::new();
    for _ in 0..2 {
        processes.push(GatedBettr::start(
            &app.database,
            &["project".to_owned(), "list".to_owned(), "--json".to_owned()],
            &gate,
        ));
    }
    gate.open(processes.len());
    for process in processes {
        process.wait();
    }

    let connection = rusqlite::Connection::open(&app.database).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 2",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 3",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 4",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 5",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events
                 WHERE operation = 'schema_migrate' AND success = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        4
    );
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
}
