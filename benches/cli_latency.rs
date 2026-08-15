const SAMPLE_COUNT: usize = 1_000;

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
