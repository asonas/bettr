# Audit JSONL verify/archive/rebuild Implementation Plan

> **Goal:** Add safe verification, SQLite-backed rebuild, and hash-preserving archive rotation for the Phase 3 JSONL audit projection.

## Constraints

- SQLite remains the canonical source; `audit_events.sequence` and `audit_jsonl_cursor` are reused.
- Use `mise exec --` for every Rust command.
- Keep `audit list`, `event list`, and existing JSON response shapes compatible.
- Add only `audit verify [--path PATH]`, `audit archive`, and `audit rebuild`.
- Do not implement redaction, retention, backup/restore, or #12 behavior.
- Preserve safe diagnostics and never expose input content or file contents.
- Do not merge, create a PR, or remove the worktree.

## Task 1: Define the CLI and error seams

Files:

- Modify `src/cli.rs`, `src/main.rs`, `src/error.rs`, `src/output.rs`.
- Modify `docs/json-contract.md` and `tests/cli_help.rs`.

Steps:

1. Add failing help/parse and JSON error contract tests for verify, archive, rebuild, and integrity failures.
2. Add the three subcommands and operation names used by invocation auditing.
3. Add stable `audit_integrity_failure` and `audit_operation_failed` errors with safe JSON details.
4. Run the focused tests through `mise exec -- cargo test --test cli_help` and the new contract test.

## Task 2: Implement strict verification

Files:

- Modify `src/store/jsonl.rs`.
- Add tests in `tests/cli_audit_tools.rs`.

Steps:

1. Write failing tests for a valid chain, hash mutation, sequence gap/duplicate, invalid JSON, and incomplete final line.
2. Parse JSONL strictly without the recovery truncation used by normal projection.
3. Recompute canonical hashes, check sequence continuity, event ID uniqueness, and previous-hash links.
4. Return a compact verification result and the new safe integrity error.
5. Run the focused integration test.

## Task 3: Implement SQLite-backed rebuild

Files:

- Modify `src/store/jsonl.rs`, `src/store/sqlite.rs`, `src/app.rs`, `src/main.rs`.
- Extend `tests/cli_audit_tools.rs`.

Steps:

1. Write a failing rebuild test that corrupts or removes active JSONL and asserts full recovery from SQLite.
2. Snapshot all audit rows under `BEGIN IMMEDIATE`, validate source sequence continuity, generate a UUID temporary file, strict-verify it, and atomically replace active.
3. Update `audit_jsonl_cursor` in the same SQLite transaction; leave the old active untouched on generation/verification failure.
4. Record the rebuild result and range through the normal audit event, with the rebuild event flushed after the reconstructed snapshot.
5. Run the focused rebuild and existing JSONL tests.

## Task 4: Implement hash-preserving archive

Files:

- Modify `src/store/jsonl.rs`, `src/store/sqlite.rs`, `src/app.rs`, `src/main.rs`.
- Extend `tests/cli_audit_tools.rs`.

Steps:

1. Write failing archive tests for generation naming, active replacement, old-hash continuation, empty/no-op behavior, and cursor mismatch.
2. Under `BEGIN IMMEDIATE`, strict-verify active, compare its tail with the SQLite cursor, atomically rename it to a unique UTC generation, and create an empty active file.
3. Keep the cursor unchanged so normal post-command projection links the next event to the archived tail.
4. Record archive success/failure through the normal audit wrapper.
5. Run the focused archive and JSONL recovery/concurrency tests.

## Task 5: Verify and hand off

1. Update docs and help to describe the error/recovery contract.
2. Run `mise exec -- cargo fmt -- --check`.
3. Run `mise exec -- cargo clippy --locked --all-targets --all-features -- -D warnings`.
4. Run `mise exec -- cargo test --locked`.
5. Commit only the requested #10 changes with an English, non-Conventional commit message.
6. Re-read #10 status/history, record the base commit, implementation commit, tests, and residual limitations in a conversation update.

