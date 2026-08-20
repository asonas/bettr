# Idempotency Keys & JSON Batch Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add transactional idempotency keys to every mutating bettr command and an atomic JSON batch API for multiple Issue mutations.

**Architecture:** Store one globally unique idempotency key per database with an operation name, SHA-256 request digest, and serialized successful result. Each individual write checks and records that table inside its existing immediate transaction; `issue batch` uses one transaction-local implementation for all supported Issue mutations and records one batch result. Replays return the stored data before any domain or audit event is written.

**Tech Stack:** Rust 2024, Clap, serde/serde_json, rusqlite, SQLite migrations, SHA-256, assert_cmd, tempfile.

**Spec:** `docs/superpowers/specs/2026-08-20-idempotency-batch-design.md`

## Global Constraints

- Keep JSON response `schema_version` at `1`; add only backward-compatible fields and the `idempotency_conflict` error code.
- Advance the SQLite schema from version 3 to version 4 transactionally; fresh databases must start at version 4.
- The idempotency key is optional, nonblank, and at most 200 Unicode scalar values.
- Store the canonical request digest without execution context, timestamps, generated UUIDs, or output mode.
- Memoize only committed successful writes; do not memoize validation, revision, domain, or database-busy failures.
- A replay must create no domain event and no audit event.
- A batch failure must roll back every Issue mutation, domain event, success audit, and idempotency record from that batch.
- Run Rust commands through `mise exec --`; run `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` before completion.

---

### Task 1: Add schema v4, stable errors, and request/result types

**Files:**
- Modify: `Cargo.toml` and `Cargo.lock`
- Modify: `src/error.rs`
- Modify: `src/domain.rs`
- Modify: `src/store/schema.sql`
- Modify: `src/store/migrations.rs`
- Test: `src/error.rs` and `src/store/migrations.rs` unit tests

**Interfaces:**
- `AppError::IdempotencyConflict` returns code `idempotency_conflict` and exit code 4.
- `domain::validate_idempotency_key(&str)` rejects blank values and values over 200 Unicode scalar values.
- `domain::BatchOperation` is a serde-tagged enum with `issue_create`, `issue_edit`, `issue_comment`, `issue_start`, `issue_block`, `issue_resume`, `issue_complete`, `issue_cancel`, and `issue_reopen` variants.
- `domain::BatchResult` serializes the operation name and its result value in the order supplied.
- Schema v4 contains `idempotency_records` and nullable `audit_events.idempotency_key`.

- [ ] **Step 1: Write the failing tests**

Add a migration assertion that starts a version-3 in-memory database, applies pending migrations, and checks the new table, unique key, and audit column. Add an error assertion:

```rust
let error = crate::error::AppError::IdempotencyConflict;
assert_eq!(error.code(), "idempotency_conflict");
assert_eq!(error.exit_code() as u8, 4);
```

Add key validation cases for `""`, whitespace, 200 characters, and 201 characters. Add a serde test that parses one `issue_edit` batch item and rejects an unknown operation.

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `mise exec -- cargo test store::migrations::tests::schema_v4_adds_idempotency_records_and_audit_key error::tests::idempotency_conflict_has_a_stable_contract domain::tests::idempotency_key_validation`

Expected: FAIL because schema version 4, the error variant, key validator, and batch enum do not exist.

- [ ] **Step 3: Implement the minimal schema and types**

Add `sha2 = "0.10"`. Set `LATEST_SCHEMA_VERSION` to `4`, register `idempotency_records` migration, add the table and audit column to the fresh schema, and update migration test fixtures. Define the error and validation contract. Derive `Serialize` and `Deserialize` for response types that must be replayed and define the tagged batch input types with `deny_unknown_fields`.

- [ ] **Step 4: Run the focused tests to verify they pass**

Run: `mise exec -- cargo test store::migrations::tests::schema_v4_adds_idempotency_records_and_audit_key error::tests::idempotency_conflict_has_a_stable_contract domain::tests::idempotency_key_validation`

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add Cargo.toml Cargo.lock src/error.rs src/domain.rs src/store/schema.sql src/store/migrations.rs
git commit -m "Add idempotency schema and request types"
```

### Task 2: Implement the transactional idempotency store and CLI key

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/output.rs`
- Create: `tests/cli_idempotency.rs`

**Interfaces:**
- `Cli::idempotency_key: Option<String>` is a global Clap argument.
- `store::IdempotencyRequest` contains `key`, `operation`, and canonical payload JSON.
- `Database::check_idempotency<T>` returns `Some(T)` only for a matching committed result, and returns `IdempotencyConflict` for an operation or digest mismatch.
- `Database::remember_idempotency<T>` inserts the key, operation, SHA-256 digest, serialized data, and timestamp in the caller's transaction.
- Mutating `App` methods accept `Option<&str>` and build request payloads from resolved arguments.

- [ ] **Step 1: Write the failing integration tests**

Create a database and project, run `issue create` twice with the same key and identical arguments, then assert equal JSON data and exactly one issue, one `issue_created` domain event, one successful `issue_create` audit, and one idempotency record. Add a changed-title reuse case and an operation-reuse case; both must exit 4 with `idempotency_conflict` and leave the original Issue unchanged.

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `mise exec -- cargo test --test cli_idempotency`

Expected: FAIL because the global key is not parsed and no idempotency record is created.

- [ ] **Step 3: Implement the transaction helper and key propagation**

Add canonical JSON serialization and SHA-256 digesting in `src/store/sqlite.rs`. At the start of each immediate write transaction, look up the key before mutation; on a match deserialize the stored data and return it without inserting audit or domain events. Before the existing success audit and commit, insert the result record. Add the key to `AuditInsert`, `audit_events`, `AuditEvent`, audit queries, and failure recording. Update `main.rs` and `App` so the key is passed through without entering the execution-context payload.

- [ ] **Step 4: Run the focused tests to verify they pass**

Run: `mise exec -- cargo test --test cli_idempotency`

Expected: PASS with one committed result and explicit conflict errors.

- [ ] **Step 5: Commit**

```sh
git add src/cli.rs src/main.rs src/app.rs src/store/sqlite.rs src/output.rs tests/cli_idempotency.rs
git commit -m "Add transactional idempotency replay"
```

### Task 3: Cover every mutating command and failure semantics

**Files:**
- Modify: `src/app.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/main.rs`
- Modify: `tests/cli_idempotency.rs`
- Modify: `tests/cli_issue_create.rs`
- Modify: `tests/cli_issue_edit.rs`
- Modify: `tests/cli_issue_transition.rs`
- Modify: `tests/cli_claim.rs`
- Modify: `tests/cli_decisions.rs`
- Modify: `tests/cli_issue_dependencies.rs`
- Modify: `tests/cli_project.rs`
- Modify: `tests/cli_init.rs`
- Modify: `tests/sqlite_concurrency.rs`

**Interfaces:**
- `project create`, `issue create/edit/comment`, dependency and parent writes, claim/heartbeat/takeover, decision request/resolve, every Issue transition, and `init` all accept the same optional key.
- Every operation's request payload includes its resolved target and expected revision when applicable.
- Revision conflicts retain `revision_conflict` and the current revision; database locks retain `database_busy` and do not leave a replay record.

- [ ] **Step 1: Write the failing coverage tests**

Add one replay test for each write family, checking returned data equality and no second domain/success-audit event. Add a stale revision test with a key, a locked database test with a key, and an audit assertion that the original successful row exposes the key while the replay creates no row. Add the existing rollback-trigger tests with a key to ensure the idempotency record rolls back with the mutation.

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `mise exec -- cargo test --test cli_idempotency --test cli_claim --test cli_decisions --test cli_issue_dependencies --test sqlite_concurrency`

Expected: FAIL for unpropagated command paths and missing failure/audit behavior.

- [ ] **Step 3: Implement the remaining write-path request identities**

Give each `App` method a stable operation payload and pass it to the matching transaction helper. Preserve existing revision checks, lease checks, decision checks, and audit allowlists. Add initialization replay for a key stored during first schema creation; an existing database with a different or absent key retains `database_already_initialized`.

- [ ] **Step 4: Run the focused tests to verify they pass**

Run: `mise exec -- cargo test --test cli_idempotency --test cli_claim --test cli_decisions --test cli_issue_dependencies --test sqlite_concurrency`

Expected: PASS, with all existing failure exit codes unchanged.

- [ ] **Step 5: Commit**

```sh
git add src/app.rs src/store/sqlite.rs src/main.rs tests/cli_idempotency.rs tests/cli_issue_create.rs tests/cli_issue_edit.rs tests/cli_issue_transition.rs tests/cli_claim.rs tests/cli_decisions.rs tests/cli_issue_dependencies.rs tests/cli_project.rs tests/cli_init.rs tests/sqlite_concurrency.rs
git commit -m "Apply idempotency to every write path"
```

### Task 4: Add atomic Issue JSON batches

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/domain.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/output.rs`
- Create: `tests/cli_issue_batch.rs`

**Interfaces:**
- `IssueSubcommand::Batch(IssueBatchCommand)` exposes `--input PATH`; `-` reads stdin.
- `App::batch_issues(input, default_project, idempotency_key, context)` returns `Vec<domain::BatchResult>`.
- `Database::batch_issues(operations, default_project, request, context)` executes supported operations inside one immediate transaction.

- [ ] **Step 1: Write the failing batch tests**

Add a successful two-item edit/transition batch and assert both Issues changed, two domain events exist, one `issue_batch` success audit exists, and the JSON result order matches input. Add a second invocation with the same key and assert byte-equivalent data and unchanged event/audit counts. Add a batch with a valid first item and invalid second item and assert all Issues, events, audits, and idempotency rows remain unchanged. Add a changed-payload key conflict case.

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `mise exec -- cargo test --test cli_issue_batch`

Expected: FAIL because the batch subcommand and transaction implementation do not exist.

- [ ] **Step 3: Implement parsing and one-transaction execution**

Add the Clap subcommand and input reader, parse the tagged JSON array, resolve project defaults, validate every item before mutation, and use transaction-local create/edit/comment/transition helpers. Insert one batch idempotency result and one `issue_batch` audit only after every item succeeds. Return the first domain or revision error after rollback.

- [ ] **Step 4: Run the focused tests to verify they pass**

Run: `mise exec -- cargo test --test cli_issue_batch`

Expected: PASS for success, replay, conflict, and rollback.

- [ ] **Step 5: Commit**

```sh
git add src/cli.rs src/main.rs src/app.rs src/domain.rs src/store/sqlite.rs src/output.rs tests/cli_issue_batch.rs
git commit -m "Add atomic Issue JSON batches"
```

### Task 5: Update capability, contract, documentation, and skills

**Files:**
- Modify: `src/app.rs`
- Modify: `contracts/capabilities.json`
- Modify: `contracts/json-schema/v1/capabilities.schema.json`
- Modify: `docs/json-contract.md`
- Modify: `README.md`
- Modify: `skills/bettr/SKILL.md`
- Modify: `skills/bettr-claude/SKILL.md`
- Modify: `tests/cli_capabilities.rs`
- Modify: `tests/skill_contracts.rs`

**Interfaces:**
- `capabilities --json` reports `idempotency: true` while preserving JSON contract version 1.
- Skill guidance requires capability discovery, documents `--idempotency-key` and `issue batch`, and explains replay, collision, revision-conflict, and busy behavior.

- [ ] **Step 1: Write the failing contract tests**

Change capability expectations to `true`, add a JSON-contract assertion for `idempotency_conflict`, and require both skills to mention the supported idempotency key and batch behavior while no longer claiming the capability is unavailable.

- [ ] **Step 2: Run the focused tests to verify they fail**

Run: `mise exec -- cargo test --test cli_capabilities --test skill_contracts`

Expected: FAIL against the current false capability and unavailable-skill text.

- [ ] **Step 3: Update the public artifacts**

Set the capability in the source and fixture, document the version-1 additive fields and error code, add batch examples and retry rules to the README/JSON contract, and synchronize the Codex and Claude adapters without advertising unsupported commands.

- [ ] **Step 4: Run the focused tests to verify they pass**

Run: `mise exec -- cargo test --test cli_capabilities --test skill_contracts`

Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add src/app.rs contracts/capabilities.json contracts/json-schema/v1/capabilities.schema.json docs/json-contract.md README.md skills/bettr/SKILL.md skills/bettr-claude/SKILL.md tests/cli_capabilities.rs tests/skill_contracts.rs
git commit -m "Publish idempotency capability and batch contract"
```

### Task 6: Full verification and handoff

**Files:**
- Modify only files required by failing verification; do not add unrelated cleanup.

- [ ] **Step 1: Inspect the final diff and worktree**

Run: `git --no-pager status --short --branch`, `git --no-pager diff origin/main...HEAD --stat`, and `git --no-pager diff --check`.

Expected: only Issue #8 files are changed, with no whitespace errors.

- [ ] **Step 2: Run formatting and lint**

Run: `mise exec -- cargo fmt --check` and `mise exec -- cargo clippy --all-targets --all-features -- -D warnings`.

Expected: both commands exit 0 without warnings.

- [ ] **Step 3: Run the complete test suite**

Run: `mise exec -- cargo test`.

Expected: all unit, CLI, concurrency, web, and frontend-adjacent Rust tests pass.

- [ ] **Step 4: Verify the public CLI manually**

Use a temporary database to run `init`, `project create`, two keyed Issue writes, a replay, a conflicting reuse, a successful batch, a failing batch, and `capabilities --json`. Confirm the response envelopes, exit codes, database counts, and audit rows match the tests.

- [ ] **Step 5: Record verification and complete bettr#8**

Fetch the latest Issue revision, add one verification comment containing the base commit, final commit, and exact passing commands, then complete the Issue with the same evidence. Do not mark it done before the full test and lint results are available.
