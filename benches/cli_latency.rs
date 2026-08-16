const SAMPLE_COUNT: usize = 1_000;
const LIST_SAMPLE_COUNT: usize = 50;
const ISSUE_COUNT: i64 = 100_000;
const BETTR_APPLICATION_ID: i64 = 0x4254_5452;

fn main() {
    let binary = bettr_binary();
    let directory = tempfile::tempdir().expect("create benchmark directory");
    let database = directory.path().join("latency.db");

    run(&binary, &database, &["init"], "prepare benchmark database");
    run(
        &binary,
        &database,
        &["project", "create", "latency"],
        "create benchmark project",
    );
    run(
        &binary,
        &database,
        &[
            "issue",
            "create",
            "--project",
            "latency",
            "--title",
            "Latency fixture",
        ],
        "create benchmark issue",
    );

    let mut reads = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started_at = std::time::Instant::now();
        run(
            &binary,
            &database,
            &["issue", "show", "1", "--project", "latency"],
            "show benchmark issue",
        );
        reads.push(started_at.elapsed());
    }

    let mut writes = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..SAMPLE_COUNT {
        let revision = (index + 1).to_string();
        let title = format!("Latency fixture {index}");
        let started_at = std::time::Instant::now();
        run(
            &binary,
            &database,
            &[
                "issue",
                "edit",
                "1",
                "--project",
                "latency",
                "--revision",
                &revision,
                "--title",
                &title,
            ],
            "update benchmark issue",
        );
        writes.push(started_at.elapsed());
    }

    report("issue show", &mut reads);
    report("issue edit", &mut writes);

    seed_large_issue_set(&database);
    let mut filtered_lists = Vec::with_capacity(LIST_SAMPLE_COUNT);
    for _ in 0..LIST_SAMPLE_COUNT {
        let started_at = std::time::Instant::now();
        run(
            &binary,
            &database,
            &[
                "issue",
                "list",
                "--project",
                "latency",
                "--state",
                "blocked",
                "--priority",
                "critical",
                "--assignee",
                "agent-12",
                "--query",
                "needle",
                "--json",
            ],
            "list selectively filtered benchmark issues",
        );
        filtered_lists.push(started_at.elapsed());
    }
    report(
        "issue list (100,000 issues, selective filters)",
        &mut filtered_lists,
    );
}

fn seed_large_issue_set(database: &std::path::Path) {
    let mut connection = rusqlite::Connection::open(database).expect("open benchmark database");
    let application_id = connection
        .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
        .expect("read benchmark application id");
    let user_version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .expect("read benchmark schema version");
    assert_eq!(application_id, BETTR_APPLICATION_ID);
    assert_eq!(user_version, 1);
    let project_id = connection
        .query_row(
            "SELECT id FROM projects WHERE name = 'latency'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read benchmark project id");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("begin benchmark seed transaction");
    {
        let mut insert = transaction
            .prepare_cached(
                "INSERT INTO issues (
                    id, project_id, number, title, body, state, priority, assignee_kind,
                    assignee_name, revision, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, 'agent', ?7, 1, ?8, ?8)",
            )
            .expect("prepare benchmark issue insert");
        for number in 2..=ISSUE_COUNT {
            let state = match number % 5 {
                0 => "todo",
                1 => "in_progress",
                2 => "blocked",
                3 => "done",
                _ => "cancelled",
            };
            let priority = match number % 4 {
                0 => "critical",
                1 => "high",
                2 => "medium",
                _ => "low",
            };
            let title = if number % 1_000 == 12 {
                format!("Needle issue {number}")
            } else {
                format!("Fixture issue {number}")
            };
            insert
                .execute(rusqlite::params![
                    format!("00000000-0000-4000-8000-{number:012x}"),
                    project_id,
                    number,
                    title,
                    state,
                    priority,
                    format!("agent-{}", number % 100),
                    "2026-01-01T00:00:00Z",
                ])
                .expect("insert benchmark issue");
        }
    }
    transaction.commit().expect("commit benchmark seed");
    let issue_count = connection
        .query_row("SELECT COUNT(*) FROM issues", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("count benchmark issues");
    assert_eq!(issue_count, ISSUE_COUNT);
}

fn bettr_binary() -> std::path::PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_bettr") {
        return path.into();
    }

    let benchmark = std::env::current_exe().expect("resolve benchmark executable");
    benchmark
        .parent()
        .and_then(std::path::Path::parent)
        .expect("resolve Cargo profile directory")
        .join(format!("bettr{}", std::env::consts::EXE_SUFFIX))
}

fn run(
    binary: &std::path::Path,
    database: &std::path::Path,
    arguments: &[&str],
    description: &str,
) {
    let output = std::process::Command::new(binary)
        .env_clear()
        .env("BETTR_OPERATOR", "latency-benchmark")
        .env(
            "HOME",
            database.parent().expect("benchmark database parent"),
        )
        .env(
            "XDG_CONFIG_HOME",
            database
                .parent()
                .expect("benchmark database parent")
                .join("config"),
        )
        .env(
            "XDG_DATA_HOME",
            database
                .parent()
                .expect("benchmark database parent")
                .join("data"),
        )
        .current_dir(database.parent().expect("benchmark database parent"))
        .arg("--database")
        .arg(database)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("{description}: {error}"));
    assert!(
        output.status.success(),
        "{description}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn report(name: &str, samples: &mut [std::time::Duration]) {
    samples.sort_unstable();
    println!(
        "{name} (n={}): p50 {:.3} ms, p95 {:.3} ms",
        samples.len(),
        milliseconds(percentile(samples, 50)),
        milliseconds(percentile(samples, 95)),
    );
}

fn percentile(samples: &[std::time::Duration], percentile: usize) -> std::time::Duration {
    let rank = (samples.len() * percentile).div_ceil(100);
    samples[rank.saturating_sub(1)]
}

fn milliseconds(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}
