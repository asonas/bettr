mod support;

fn context_command(home: &std::path::Path) -> assert_cmd::Command {
    let mut command = assert_cmd::Command::cargo_bin("bettr").unwrap();
    command
        .env_clear()
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .args(["context", "--json"]);
    command
}

fn context_data(output: std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "context failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["data"].clone()
}

fn user_config_path(home: &std::path::Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/bettr/config.toml")
    } else {
        home.join("config/bettr/config.toml")
    }
}

fn default_database_path(home: &std::path::Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/bettr/bettr.db")
    } else {
        home.join("data/bettr/bettr.db")
    }
}

fn write_user_config(home: &std::path::Path, contents: &str) {
    let path = user_config_path(home);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn context_resolves_project_in_argument_environment_directory_user_and_default_order() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let workspace = directory.path().join("workspace/nested");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    write_user_config(&home, "project = \"user-project\"\n");
    std::fs::write(
        directory.path().join("workspace/.bettr.toml"),
        "project = \"directory-project\"\n",
    )
    .unwrap();

    let argument = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .env("BETTR_PROJECT", "environment-project")
            .args(["--project", "argument-project"])
            .output()
            .unwrap(),
    );
    assert_eq!(argument["project"]["value"], "argument-project");
    assert_eq!(argument["project"]["source"], "argument");

    let environment = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .env("BETTR_PROJECT", "environment-project")
            .output()
            .unwrap(),
    );
    assert_eq!(environment["project"]["value"], "environment-project");
    assert_eq!(environment["project"]["source"], "environment");

    let directory_context = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .output()
            .unwrap(),
    );
    assert_eq!(directory_context["project"]["value"], "directory-project");
    assert_eq!(directory_context["project"]["source"], "directory_config");

    std::fs::remove_file(directory.path().join("workspace/.bettr.toml")).unwrap();
    let user = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .output()
            .unwrap(),
    );
    assert_eq!(user["project"]["value"], "user-project");
    assert_eq!(user["project"]["source"], "user_config");

    std::fs::remove_file(user_config_path(&home)).unwrap();
    let default = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .output()
            .unwrap(),
    );
    assert!(default["project"]["value"].is_null());
    assert_eq!(default["project"]["source"], "default");
}

#[test]
fn context_resolves_database_in_argument_environment_user_and_os_default_order() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let user_database = directory.path().join("user.db");
    write_user_config(
        &home,
        &format!("database = {:?}\n", user_database.to_string_lossy()),
    );

    let argument_database = directory.path().join("argument.db");
    let environment_database = directory.path().join("environment.db");
    let argument = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .env("BETTR_DATABASE", &environment_database)
            .args(["--database", argument_database.to_str().unwrap()])
            .output()
            .unwrap(),
    );
    assert_eq!(
        argument["database"]["value"],
        argument_database.to_str().unwrap()
    );
    assert_eq!(argument["database"]["source"], "argument");

    let environment = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .env("BETTR_DATABASE", &environment_database)
            .output()
            .unwrap(),
    );
    assert_eq!(
        environment["database"]["value"],
        environment_database.to_str().unwrap()
    );
    assert_eq!(environment["database"]["source"], "environment");

    let user = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .output()
            .unwrap(),
    );
    assert_eq!(user["database"]["value"], user_database.to_str().unwrap());
    assert_eq!(user["database"]["source"], "user_config");

    std::fs::remove_file(user_config_path(&home)).unwrap();
    let default = context_data(
        context_command(&home)
            .current_dir(&workspace)
            .output()
            .unwrap(),
    );
    assert_eq!(
        default["database"]["value"],
        default_database_path(&home).to_str().unwrap()
    );
    assert_eq!(default["database"]["source"], "default");
}

#[test]
fn context_does_not_create_the_resolved_database() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let database = directory.path().join("missing/nested/bettr.db");

    let data = context_data(
        context_command(&home)
            .args(["--database", database.to_str().unwrap()])
            .output()
            .unwrap(),
    );

    assert_eq!(data["database"]["value"], database.to_str().unwrap());
    assert!(!database.exists());
    assert!(!database.parent().unwrap().exists());
}

#[test]
fn invalid_or_ambiguous_configuration_exits_two() {
    let cases = [
        "project = \"one\"\nproject = \"two\"\n",
        "project = 7\n",
        "unknown = \"value\"\n",
        "project = \"   \"\n",
    ];

    for contents in cases {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        write_user_config(&home, contents);

        let output = context_command(&home)
            .current_dir(directory.path())
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(2));
        let response: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
    }
}

#[test]
fn shared_test_commands_isolate_home_xdg_and_current_directory() {
    let app = crate::support::TestApp::new();
    let command = app.command();
    let environment = command
        .get_envs()
        .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value.to_owned())))
        .collect::<std::collections::HashMap<_, _>>();

    assert_eq!(
        command.get_current_dir(),
        Some(app.dir.path().join("work").as_path())
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("HOME")),
        Some(&app.dir.path().join("home").into_os_string())
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("XDG_CONFIG_HOME")),
        Some(&app.dir.path().join("config").into_os_string())
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("XDG_DATA_HOME")),
        Some(&app.dir.path().join("data").into_os_string())
    );
}
