use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_names_the_product_and_core_commands() {
    Command::cargo_bin("bettr")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Local issue tracking for agent work",
        ))
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("project"))
        .stdout(predicate::str::contains("issue"))
        .stdout(predicate::str::contains("decision"))
        .stdout(predicate::str::contains("event"))
        .stdout(predicate::str::contains("capabilities"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("web"));

    Command::cargo_bin("bettr")
        .unwrap()
        .args(["web", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--port"));

    Command::cargo_bin("bettr")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
