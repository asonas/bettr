# Issue #15 Skills and Wayfinder Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Phase 2 coordination and event contracts required for Codex, Claude Code, and wayfinder to use bettr without command guessing.

**Architecture:** Extend the existing SQLite schema through one migration, keep atomic coordination operations in `Database`, expose validation and response types through `App` and the existing Clap CLI, and keep runtime skill packages as thin adapters over `capabilities --json`. Domain events remain the cursor source; lease heartbeats update lease timing without adding noisy cursor events.

**Tech Stack:** Rust 2024, Clap, Serde/serde_json, rusqlite, SQLite WAL, Cargo integration tests, Markdown skill packages.

**Spec:** `docs/superpowers/specs/2026-08-16-issue-15-skills-wayfinder-design.md`

**Implementation status (2026-08-16):** Tasks 1–7 are implemented in the Issue #15 worktree. The binary-only Cargo package uses `--bin bettr` for unit-test filters; the final release gate remains below.

## Global Constraints

- Preserve the versioned JSON envelope: every response has `schema_version` and exactly one of `data` or `error`.
- Use SQLite immediate transactions for every multi-record coordination write.
- Use the existing revision guard for Issue data; never silently overwrite a concurrent update.
- Use `BETTR_AGENT` and `BETTR_SESSION_ID` for agent execution context.
- Keep lease TTL fixed at 15 minutes and do not auto-transition stale Issues.
- Do not add a daemon, network protocol, authentication system, or idempotency implementation in this plan.
- Every production behavior change gets a failing integration or unit test before implementation.
- Run Rust commands through `mise exec --` and use `cargo clippy --all-targets --all-features -- -D warnings` before handoff.

### Task 1: Record the design and establish the contract fixtures

**Files:**
- Create: `contracts/capabilities.json`
- Create: `contracts/json-schema/v1/capabilities.schema.json`
- Create: `contracts/json-schema/v1/event-list.schema.json`
- Create: `contracts/json-schema/v1/coordination.schema.json`
- Modify: `docs/json-contract.md`
- Modify: `docs/implementation-roadmap.md`
- Test: `tests/cli_help.rs`

**Interfaces:**
- Produces the capability names used by Rust and both skill packages.
- Produces stable fixture shapes for `capabilities`, coordination responses, and event pages.

- [ ] **Step 1: Write the capability matrix and schema fixtures**

Declare `issue_dependencies`, `issue_claim`, `issue_lease`, `human_decisions`, `event_cursor`, and `capabilities` as Phase 2 capabilities. Mark `idempotency` as unavailable until its separate implementation exists. Keep schema version 1 explicit.

- [ ] **Step 2: Add the failing help assertion**

Extend the existing CLI help test to require the new top-level `decision`, `event`, and `capabilities` command names.

- [ ] **Step 3: Run the focused test and verify it fails**

Run: `mise exec -- cargo test --test cli_help`

Expected: FAIL because the new commands are not yet registered.

- [ ] **Step 4: Update the contract documentation**

Document additive fields, capability gating, exclusive cursors, and the distinction between Issue revision and lease timing.

- [ ] **Step 5: Re-run the focused test**

Run: `mise exec -- cargo test --test cli_help`

Expected: the existing help assertions remain green; the new assertions stay red until Task 6 registers the commands.

### Task 2: Add Schema v3 for dependencies, leases, and decision requests

**Files:**
- Modify: `src/store/schema.sql`
- Modify: `src/store/migrations.rs`
- Test: `tests/cli_init.rs`
- Test: `src/store/migrations.rs`

**Interfaces:**
- Produces schema version 3 with `issue_dependencies`, `issue_parents`, `issue_leases`, and `decision_requests`.
- Keeps schema v1 and v2 migration paths intact.

- [ ] **Step 1: Write migration tests**

Add a v2-to-v3 migration test that checks all four tables, indexes, and `user_version = 3`, plus a rollback test that leaves version 2 unchanged when a migration statement fails.

- [ ] **Step 2: Run the migration tests and verify failure**

Run: `mise exec -- cargo test store::migrations::tests --lib`

Expected: FAIL because the latest version is still 2 and no Phase 2 tables exist.

- [ ] **Step 3: Implement the migration**

Add one transactional migration that creates the tables and indexes, records the migration name, and updates `LATEST_SCHEMA_VERSION` to 3. Keep foreign keys and one-parent constraints in SQLite.

- [ ] **Step 4: Add CLI migration coverage**

Extend the initialized-database tests to verify a v2 database upgrades to v3 without changing existing Issues or domain events.

- [ ] **Step 5: Run migration and initialization tests**

Run: `mise exec -- cargo test --bin bettr store::migrations::tests && mise exec -- cargo test --test cli_init`

Expected: PASS with schema version 3 and no data loss.

### Task 3: Implement dependency and parent relations

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/domain.rs`
- Modify: `src/store/sqlite.rs`
- Test: `tests/cli_issue_dependencies.rs`

**Interfaces:**
- Adds `issue dependency add|remove|list` and `issue parent set|list`.
- Adds `Database::add_dependency`, `remove_dependency`, `list_dependencies`, `set_parent`, and `list_parent` operations.
- Returns structured relation objects with project, Issue number, relation type, and timestamps.

- [ ] **Step 1: Write tests for relation creation and listing**

Cover directed `blocks`, duplicate rejection, self-link rejection, one parent per child, cross-project references, and JSON envelopes.

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `mise exec -- cargo test --test cli_issue_dependencies`

Expected: FAIL because the subcommands and tables are absent.

- [ ] **Step 3: Add domain validation and CLI parsing**

Parse `PROJECT#NUMBER` references, use global `--project` for unqualified numbers, keep the relation direction explicit, and reject blank or self-referential links before opening a write transaction.

- [ ] **Step 4: Implement atomic store operations and cycle checks**

Use a recursive query inside an immediate transaction to reject a path from the candidate blocked Issue back to the blocker. Reject a parent whose own parent exists, preserving the one-level rule.

- [ ] **Step 5: Run the focused tests and inspect JSON**

Run: `mise exec -- cargo test --test cli_issue_dependencies`

Expected: PASS; failed writes leave relation tables unchanged.

### Task 4: Implement atomic claim, lease, heartbeat, and takeover

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/domain.rs`
- Modify: `src/store/sqlite.rs`
- Test: `tests/cli_claim.rs`
- Test: `tests/sqlite_concurrency.rs`

**Interfaces:**
- Adds `issue claim [NUMBER]`, `issue heartbeat NUMBER`, and `issue takeover NUMBER --reason`.
- Returns the Issue plus lease owner, `claimed_at`, `heartbeat_at`, and `expires_at`.
- Claim selection excludes non-todo Issues, unresolved blockers, and Issues with open decision requests.

- [ ] **Step 1: Write claim and lease tests**

Cover explicit and automatic claim, priority ordering, dependency filtering, open-decision filtering, one-claim race, wrong-session heartbeat, expiry without state mutation, takeover reason validation, and revision increments for claim/takeover only.

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `mise exec -- cargo test --test cli_claim`

Expected: FAIL because the new command and lease table are absent.

- [ ] **Step 3: Implement lease domain types and fixed TTL**

Use a 15-minute duration, serialize UTC timestamps, require agent context for claim/heartbeat/takeover, and keep heartbeat out of Issue revision and domain cursor events.

- [ ] **Step 4: Implement selection and atomic writes**

Start one immediate transaction, select the eligible Issue in deterministic priority/creation order, update assignment/state/revision, insert or replace the lease, and append one domain/audit event before commit.

- [ ] **Step 5: Implement ownership and takeover checks**

Require matching agent and session for heartbeat, reject active leases from other sessions, allow takeover only after expiry, and store the takeover reason in audit metadata without exposing it in changed fields.

- [ ] **Step 6: Run concurrency and lease tests**

Run: `mise exec -- cargo test --test cli_claim --test sqlite_concurrency`

Expected: exactly one concurrent claimant succeeds; stale leases remain `in_progress`, appear in the status stale section, and are never reassigned automatically.

### Task 5: Implement human decision requests and resolution

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/domain.rs`
- Modify: `src/store/sqlite.rs`
- Test: `tests/cli_decisions.rs`
- Modify: `tests/cli_status.rs`

**Interfaces:**
- Adds `decision request NUMBER --question QUESTION --background BACKGROUND`.
- Adds `decision resolve REQUEST_ID --answer ANSWER --next-state STATE`, with state-specific transition metadata: `--reason` and `--wait-kind` for `blocked`, `--summary` and `--verification` for `done`, and `--reason` for `cancelled`.
- Adds serialized request objects and `attention_required` data to status/Issue responses.

- [ ] **Step 1: Write request and resolution tests**

Cover multiple open requests, automatic block with human wait metadata, requester context, human resolver context, unresolved-request completion rejection, wrong-session resolution rejection, and atomic state/request updates.

- [ ] **Step 2: Run the focused tests and verify failure**

Run: `mise exec -- cargo test --test cli_decisions --test cli_status`

Expected: FAIL because decision commands and request storage do not exist.

- [ ] **Step 3: Add decision domain types and validation**

Validate nonblank question, background, and answer values; represent open/resolved status; reject resolution by the requesting agent session; require an explicit next state.

- [ ] **Step 4: Implement atomic request creation and resolution**

Create a request and block the Issue in one transaction. Resolve the request, record answer and human context, reject `done` while another request remains open, validate state-specific transition metadata, and append the corresponding safe domain/audit events.

- [ ] **Step 5: Add attention-required status projection**

Load open-request presence into status and Issue coordination responses without changing the five persisted Issue states.

- [ ] **Step 6: Run decision and status tests**

Run: `mise exec -- cargo test --test cli_decisions --test cli_status`

Expected: PASS with all request/state updates committed atomically.

### Task 6: Add event cursor and capability discovery

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/domain.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/output.rs`
- Test: `tests/cli_events.rs`
- Test: `tests/cli_capabilities.rs`

**Interfaces:**
- Adds `event list --after CURSOR [--limit LIMIT] [--include-issue]`.
- Adds `capabilities --json`.
- Produces `EventPage { next_cursor, has_more, events }` and `CapabilitySet { json_contract_version, cli_version, capabilities }`.

- [ ] **Step 1: Write cursor and capability tests**

Cover exclusive cursor behavior, empty pages, limit validation, invalid cursors, event ordering, optional post-change Issue, omission of heartbeat/read/failure events, one-snapshot consistency, and capability JSON shape.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `mise exec -- cargo test --test cli_events --test cli_capabilities`

Expected: FAIL because neither command exists.

- [ ] **Step 3: Add safe event and capability domain types**

Expose only allowlisted changed fields and target identifiers. Keep event metadata free of Issue/comment bodies and return additive capability booleans from the shared matrix.

- [ ] **Step 4: Implement cursor reads**

Read `domain_events` in one transaction with `sequence > after`, bounded limit, and deterministic ordering. Fetch optional post-change Issues using the same transaction and return the last observed cursor.

- [ ] **Step 5: Implement capability output**

Return the CLI version from Cargo metadata, JSON contract version 1, and the capabilities implemented in Tasks 3–6; leave idempotency false/unavailable.

- [ ] **Step 6: Run focused tests and compare fixtures**

Run: `mise exec -- cargo test --test cli_events --test cli_capabilities`

Expected: PASS and fixture output matches `contracts/capabilities.json`.

### Task 7: Add runtime skills, examples, and forward tests

**Files:**
- Modify: `skills/bettr/SKILL.md`
- Modify: `skills/bettr/agents/openai.yaml`
- Create: `skills/bettr-claude/SKILL.md`
- Create: `skills/bettr-claude/README.md`
- Modify: `README.md`
- Create: `examples/wayfinder/phase2-workflow.md`
- Create: `tests/skill_contracts.rs`

**Interfaces:**
- Both skill packages describe the same capability-gated workflow.
- The example consumes `capabilities`, claim, decision, and event cursor JSON without depending on a wayfinder binary.

- [ ] **Step 1: Write the skill contract test**

Load both skill files and assert every command/capability token is declared in `contracts/capabilities.json`; assert the files explain revision conflict handling, decision stop behavior, and event cursor polling.

- [ ] **Step 2: Run the contract test and verify failure**

Run: `mise exec -- cargo test --test skill_contracts`

Expected: FAIL because the Claude package and shared capability references are absent.

- [ ] **Step 3: Update the Codex skill and add the Claude adapter**

Keep shared CLI semantics identical, place runtime-specific installation notes in each package, and explicitly state that unsupported capabilities must not be invoked.

- [ ] **Step 4: Add the wayfinder example and install documentation**

Document a polling loop, cursor persistence, claim/decision handling, bounded retries, and immutable-ref installation checks. Keep GitHub Releases as a separate follow-up.

- [ ] **Step 5: Run the skill contract test**

Run: `mise exec -- cargo test --test skill_contracts`

Expected: PASS with no command advertised outside the capability matrix.

### Task 8: Run the release gate and record evidence

**Files:**
- Modify: `docs/json-contract.md`
- Modify: `docs/design.md`
- Modify: `docs/implementation-roadmap.md`
- Test: all existing and new Rust tests

- [ ] **Step 1: Run formatting and lint checks**

Run: `mise exec -- cargo fmt --check` and `mise exec -- cargo clippy --all-targets --all-features -- -D warnings`.

Expected: both exit successfully with no warnings.

- [ ] **Step 2: Run the full test suite with localhost permission**

Run: `mise exec -- cargo test`.

Expected: all unit, CLI, concurrency, Web API, and new Phase 2 tests pass.

- [ ] **Step 3: Run the release build**

Run: `mise exec -- cargo build --release`.

Expected: release binary builds successfully.

- [ ] **Step 4: Review requirements against the spec**

Check every Issue #15 acceptance criterion and record any deferred idempotency or release-distribution work explicitly.

- [ ] **Step 5: Record verification in Issue #15**

Add an immutable progress/completion comment containing the worktree branch, changed capability set, and fresh verification results without embedding raw command lines or secrets.
