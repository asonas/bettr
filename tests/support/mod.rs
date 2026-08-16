pub struct TestApp {
    #[allow(dead_code)]
    pub dir: tempfile::TempDir,
    pub database: std::path::PathBuf,
}

impl TestApp {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("bettr.db");
        for directory in ["home", "config", "data", "work"] {
            std::fs::create_dir(dir.path().join(directory)).unwrap();
        }
        Self { dir, database }
    }

    pub fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::Command::cargo_bin("bettr").unwrap();
        command.arg("--database").arg(&self.database);
        command.env_clear();
        command
            .current_dir(self.dir.path().join("work"))
            .env("HOME", self.dir.path().join("home"))
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"));
        command
    }
}
