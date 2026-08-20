mod support;

#[test]
fn capabilities_report_the_contract_and_match_fixture() {
    let app = crate::support::TestApp::new();
    let output = app
        .command()
        .args(["capabilities", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let capabilities = &response["data"];
    assert_eq!(capabilities["json_contract_version"], 1);
    assert_eq!(capabilities["cli_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(capabilities["capabilities"]["issue_dependencies"], true);
    assert_eq!(capabilities["capabilities"]["issue_worktrees"], true);
    assert_eq!(capabilities["capabilities"]["issue_parent"], true);
    assert_eq!(capabilities["capabilities"]["issue_claim"], true);
    assert_eq!(capabilities["capabilities"]["issue_lease"], true);
    assert_eq!(capabilities["capabilities"]["human_decisions"], true);
    assert_eq!(capabilities["capabilities"]["event_cursor"], true);
    assert_eq!(capabilities["capabilities"]["capabilities"], true);
    assert_eq!(capabilities["capabilities"]["idempotency"], true);
    assert_eq!(capabilities["capabilities"]["audit_jsonl"], true);
    assert_eq!(capabilities["capabilities"]["redaction"], true);
    assert_eq!(capabilities["capabilities"]["sqlite_backup_restore"], true);

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../contracts/capabilities.json")).unwrap();
    assert_eq!(
        capabilities["json_contract_version"],
        fixture["json_contract_version"]
    );
    assert_eq!(capabilities["capabilities"], fixture["capabilities"]);
}
