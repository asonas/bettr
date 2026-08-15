pub struct TestApp {
    #[allow(dead_code)]
    pub dir: tempfile::TempDir,
    pub database: std::path::PathBuf,
}

impl TestApp {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("bettr.db");
        Self { dir, database }
    }

    pub fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::Command::cargo_bin("bettr").unwrap();
        command.arg("--database").arg(&self.database);
        command.env_clear();
        command
    }
}
