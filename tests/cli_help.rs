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
        .stdout(predicate::str::contains("status"));
}
