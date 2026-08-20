# JSONL Audit Log Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Project every SQLite audit event into a local append-only JSONL log during normal CLI execution, with a hash chain, serialized multi-process writes, and next-run recovery.

**Architecture:** SQLite remains the canonical audit store. `audit_events.sequence` is the exclusive source cursor, and a singleton `audit_jsonl_cursor` row stores the last committed sequence and hash. A CLI run flushes the source events under `BEGIN IMMEDIATE`, repairs only an incomplete trailing JSONL line, appends missing complete events with `sync_data`, then advances the cursor in the same SQLite transaction. The log path is derived from the explicit database path by replacing its extension with `.audit.jsonl`.

**Tech Stack:** Rust 2024, serde/serde_json, rusqlite, SQLite migrations, SHA-256, assert_cmd, tempfile.

**Spec:** `docs/superpowers/specs/2026-08-20-jsonl-audit-design.md`

## Global Constraints

- Run every Rust command through `mise exec --`.
- Advance the schema from version 4 to version 5 transactionally; fresh databases must contain the sequence column and cursor table.
- Preserve the existing JSON response contracts and `audit list` / `event list` behavior.
- Keep SQLite as the source of truth; JSONL I/O failure must not change the domain result or advance the cursor.
- Emit one safe JSON object per line for successful and failed reads and writes. Never serialize raw argv, Issue/body/comment content, full response payloads, or secret values.
- Use deterministic JSON and lower-case SHA-256 for the hash input excluding the `hash` field, while including `previous_hash`.
- Do not add `audit verify`, archive, rebuild, redaction, retention, or other #10/#12 behavior.
- Do not merge, create a PR, or remove the worktree.
- Run focused RED/GREEN tests after each behavior-sized change; run fmt, clippy, and the full locked suite before handoff.

### Task 1: Add the v5 audit sequence and JSONL cursor schema

**Files:**
- Modify: `src/store/schema.sql`
- Modify: `src/store/migrations.rs`
- Test: `src/store/migrations.rs`

**Interfaces:**
- `LATEST_SCHEMA_VERSION` becomes `5` and `is_supported_version(5)` is true.
- Fresh `audit_events` rows have a unique, monotonically assigned `sequence`.
- `audit_jsonl_cursor` has one row with `id = 1`, `sequence >= 0`, nullable `previous_hash`, and `updated_at`.
- A v4 database is migrated without losing audit rows; existing rows receive deterministic sequence values.

- [ ] **Step 1: Write failing migration tests**

  Add tests for a fresh v5 schema and for a v4 fixture containing multiple audit rows. Assert the version, `audit_events.sequence`, its uniqueness, the cursor table, the seeded cursor row, and the backfilled sequence order.

- [ ] **Step 2: Run the focused tests and verify RED**

  Run `mise exec -- cargo test store::migrations::tests::schema_v5_adds_jsonl_cursor store::migrations::tests::schema_v4_backfills_audit_sequence` and confirm failure against version 4.

- [ ] **Step 3: Implement the migration and fresh schema**

  Add migration 5. On v4, add a nullable sequence column, fill it in stable `rowid` order, create a unique index, and create/seed the cursor table. Keep the migration inside the existing immediate transaction and make it safe for the current fixtures. Add the same structures to `schema.sql`.

- [ ] **Step 4: Run the focused tests and verify GREEN**

  Re-run the focused migration tests and the existing migration unit tests.

- [ ] **Step 5: Commit the schema boundary**

  Commit only the migration/schema changes with an English, non-Conventional message.

### Task 2: Give audit rows a serialized source sequence and define safe JSONL events

**Files:**
- Modify: `src/store/sqlite.rs`
- Modify: `src/app.rs`
- Create or modify: the smallest audit JSONL module needed by the existing module layout
- Test: `src/store/sqlite.rs` and the new JSONL-focused integration test

**Interfaces:**
- The central `insert_audit_event` allocates the next `audit_events.sequence` inside the caller's existing immediate transaction.
- Existing audit query output remains unchanged; JSONL projection reads source rows by exclusive sequence.
- The JSONL event contains only `schema_version`, `sequence`, `event_id`, timestamps, operation, safe context, safe project/target/revision fields, changed fields, result, `previous_hash`, and `hash`.

- [ ] **Step 1: Write failing source-sequence and serialization tests**

  Add a test that records a success and failure and asserts distinct ordered sequences. Add serialization assertions for schema version, context/result fields, safe omission of raw payload fields, deterministic hash input, and lower-case 64-character hashes.

- [ ] **Step 2: Run focused tests and verify RED**

  Run the new unit/integration tests. They must fail because audit rows have no sequence-backed JSONL projection.

- [ ] **Step 3: Implement sequence allocation and event conversion**

  Extend only the central audit insert path so every existing caller gets a sequence. Add source-row loading by `sequence > cursor.sequence`, conversion to the safe event shape, deterministic serialization, and SHA-256 chaining. Keep `metadata_json`, idempotency keys, project names, response bodies, and user-supplied text out of the exported event unless they are already represented by the allowlisted changed-field names.

- [ ] **Step 4: Run focused tests and verify GREEN**

  Run the new tests plus existing audit, event-cursor, and idempotency suites.

- [ ] **Step 5: Commit the source/event model**

  Commit the sequence and safe event model separately from CLI lifecycle wiring.

### Task 3: Flush JSONL automatically at the CLI lifecycle boundary

**Files:**
- Modify: `src/main.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/app.rs` only if a small lifecycle helper is needed
- Test: `tests/cli_jsonl_audit.rs`

**Interfaces:**
- Normal CLI execution automatically flushes the database-derived `.audit.jsonl` after the command result, including command failures that reach the application layer.
- Parseable invocation failures use the existing safe `AuditInvocation` context and also attempt a flush when the database can be opened.
- A flush error is reported safely without printing argv, request bodies, comments, or secrets; the command's original success/failure result remains authoritative.

- [ ] **Step 1: Write failing CLI tests**

  Create a temporary initialized database and exercise a successful read, successful write, invalid read/write, and a command that returns a domain error. Assert one JSON object per audit event, safe fields only, success/failure results, and no raw sensitive values.

- [ ] **Step 2: Run `tests/cli_jsonl_audit.rs` and verify RED**

  Run `mise exec -- cargo test --test cli_jsonl_audit`; it must fail because no JSONL file is produced.

- [ ] **Step 3: Implement the run wrapper and flush operation**

  Wrap the existing command dispatch in a result-preserving lifecycle closure. After it finishes, open/flush the same resolved database path while the database handle is available. Use `BEGIN IMMEDIATE` to serialize cursor selection, partial-tail repair, duplicate-tail handling, append, `sync_data`, cursor update, and commit. If the active file was rotated or deleted, continue from the cursor's previous hash; do not implement automatic archive management.

- [ ] **Step 4: Run focused CLI tests and verify GREEN**

  Re-run `cli_jsonl_audit` and the existing CLI audit/error tests. Confirm the original command exit code and JSON response are unchanged when the log is writable.

- [ ] **Step 5: Commit automatic projection**

  Commit the lifecycle and basic JSONL output behavior.

### Task 4: Test crash recovery, rotation boundaries, and concurrent appenders

**Files:**
- Modify: `src/store/sqlite.rs` or the JSONL module
- Modify: `tests/cli_jsonl_audit.rs`
- Modify: `tests/sqlite_concurrency.rs` only if a shared helper is required

**Interfaces:**
- A partial trailing line is truncated before retry; a complete matching tail event is not duplicated.
- A mismatched complete tail or hash conflict fails safely and leaves the cursor unchanged.
- Removing/renaming the active file does not reset the chain; the next file starts with the cursor's previous hash.
- Multiple processes produce contiguous, non-duplicated sequence lines and a cursor matching the final event.

- [ ] **Step 1: Add failing recovery and concurrency tests**

  Seed audit events, create partial and complete tails, rotate the active path, and launch concurrent CLI writers against the same database. Assert the exact line count, contiguous sequences, chain linkage, cursor state, and recovery on the next invocation.

- [ ] **Step 2: Run focused tests and verify RED**

  Run the recovery and concurrency tests before the hardening implementation.

- [ ] **Step 3: Implement the atomic append/recovery boundary**

  Read only the final line needed to identify a complete matching event; reject conflicting tails rather than silently repairing history. Write through a temporary/truncate-safe path as defined in the spec, call `sync_data` before updating SQLite, and keep the cursor transaction open until the append is durable enough for the documented crash cases.

- [ ] **Step 4: Run focused tests and verify GREEN**

  Re-run JSONL recovery/concurrency tests and the existing `sqlite_concurrency` suite.

- [ ] **Step 5: Commit recovery guarantees**

  Commit the crash/recovery/concurrency implementation and tests.

### Task 5: Publish the capability and document the Phase 3 boundary

**Files:**
- Modify: `src/app.rs`
- Modify: `contracts/capabilities.json`
- Modify: `contracts/json-schema/v1/capabilities.schema.json` if required by the fixture contract
- Modify: `docs/design.md`
- Modify: `docs/implementation-roadmap.md`
- Modify: `skills/bettr/SKILL.md`
- Modify: `skills/bettr-claude/SKILL.md`
- Modify: `tests/cli_capabilities.rs`
- Modify: `tests/skill_contracts.rs`

**Interfaces:**
- `capabilities --json` reports `audit_jsonl: true` without changing JSON contract version 1.
- Documentation describes automatic projection, cursor/hash/recovery guarantees, and explicit non-goals for #10/#12.

- [ ] **Step 1: Add failing capability/contract assertions**

  Require `audit_jsonl` in the capability response and fixture, and require both skill documents to describe the supported Phase 3 behavior and its non-goals.

- [ ] **Step 2: Run focused tests and verify RED**

  Run `mise exec -- cargo test --test cli_capabilities --test skill_contracts`.

- [ ] **Step 3: Update source, contracts, docs, and skills**

  Add only the capability and operational guidance needed for the implemented feature. Do not add verify/archive/rebuild/redaction commands.

- [ ] **Step 4: Run focused tests and verify GREEN**

  Re-run the focused contract tests and the JSONL integration suite.

- [ ] **Step 5: Commit the public contract**

  Commit only the capability/documentation/skill changes.

### Task 6: Final verification and Issue handoff

**Files:**
- No implementation changes; only Issue history and final verification output.

- [ ] **Step 1: Run formatting and static checks**

  Run `mise exec -- cargo fmt -- --check` and `mise exec -- cargo clippy --locked --all-targets --all-features -- -D warnings`.

- [ ] **Step 2: Run the full locked Rust suite**

  Run `mise exec -- cargo test --locked` and retain the exit status/output summary.

- [ ] **Step 3: Inspect the final diff and worktree**

  Check `git diff --check`, `git status`, and the commit list. Confirm no user changes were overwritten and no merge/PR/worktree removal occurred.

- [ ] **Step 4: Record verification on bettr#9**

  Add a structured conversation update with the baseline commit, implementation commits, test commands/results, and explicit remaining work or exclusions. Re-check Issue #9 state/history and keep it `in_progress` unless all in-scope completion criteria are actually satisfied.

- [ ] **Step 5: Commit any final implementation changes and hand off**

  Ensure the final implementation commit(s) contain only the requested Phase 3 changes. Do not merge into `main` or create a PR.
