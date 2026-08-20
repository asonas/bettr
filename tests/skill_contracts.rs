fn skill_text(path: &str) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn codex_and_claude_skills_use_only_declared_phase_two_contracts() {
    let matrix: serde_json::Value =
        serde_json::from_str(include_str!("../contracts/capabilities.json")).unwrap();
    let codex = skill_text("skills/bettr/SKILL.md");
    let claude = skill_text("skills/bettr-claude/SKILL.md");
    let combined = format!("{codex}\n{claude}");

    for capability in matrix["capabilities"]
        .as_object()
        .unwrap()
        .iter()
        .filter_map(|(name, enabled)| enabled.as_bool().filter(|enabled| *enabled).map(|_| name))
    {
        assert!(
            combined.contains(capability),
            "skill files must mention declared capability {capability}"
        );
    }
    assert!(combined.contains("idempotency-key"));
    assert!(combined.contains("bettr issue batch"));
    for command in [
        "bettr capabilities --json",
        "bettr issue claim",
        "bettr issue heartbeat",
        "bettr issue takeover",
        "bettr decision request",
        "bettr decision resolve",
        "bettr event list --after",
    ] {
        assert!(
            combined.contains(command),
            "missing command contract: {command}"
        );
    }
    for text in [
        "revision conflict",
        "stop when blocked",
        "BETTR_SESSION_ID",
        "cursor",
        "capability",
        "Blocker:",
        "Human decision:",
        "Options:",
        "Recommendation:",
        "Resume condition:",
        "--blocker",
        "--option",
        "--recommendation",
        "--resume-condition",
        "exactly one request",
    ] {
        assert!(combined.to_lowercase().contains(&text.to_lowercase()));
    }
}
