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
        let mut command = helper_command("run_bettr_after_parent_opens_start_gate");
        command
            .env(GATED_DATABASE, database)
            .env(GATED_ARGUMENTS, serde_json::to_string(arguments).unwrap());
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

    let output = bettr_command(std::path::Path::new(&database_path))
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "gated bettr failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
