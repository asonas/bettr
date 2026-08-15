# Phase 1 Local Issue Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ローカルSQLite上でプロジェクト、Issue、コメント、状態遷移、履歴、監査を安全に読み書きできる非対話式`bettr` CLIを完成させます。

**Architecture:** 単一Cargo packageを、CLI境界、アプリケーション操作、ドメイン型、SQLite永続化に分けます。書き込みはアプリケーション操作ごとの短いトランザクションで現在値と追記専用イベントを同時に保存し、Issueの更新はrevisionを使った楽観的ロックで保護します。

**Tech Stack:** Rust 2024 edition、clap derive、rusqlite bundled、serde/serde_json、uuid、chrono、thiserror、assert_cmd、predicates、tempfile

**Spec:** `docs/design.md`

## Global Constraints

Rustコマンドは実行前に`command -v rustc`と`rustc --version`を確認し、実行には`mise exec -- cargo ...`を使います。

正式対応OSはmacOSとLinuxです。Windows、HTTPサーバー、常駐デーモン、ネットワーク共有、SQLite以外のデータベースは実装しません。

CLIは非対話式とし、標準出力には結果だけ、標準エラーには診断だけを出します。時刻はDBとJSONではUTC RFC 3339、端末ではローカル時刻で扱います。

SQLiteはWAL、foreign keys、5秒のbusy timeoutを接続ごとに設定します。書き込みトランザクション内で外部I/Oを行いません。

通常の状態は`todo`、`in_progress`、`blocked`、`done`、`cancelled`です。通常遷移は`todo -> in_progress`、`in_progress -> blocked|done|cancelled`、`blocked -> in_progress|cancelled`、`done|cancelled -> todo`だけを許可します。

すべてのCLI呼び出しは、成功と失敗、読み取りと書き込みを含めてSQLite内の監査イベントへ記録します。本文、コメント本文、生のコマンドラインは監査payloadへ保存しません。

コミットには、そのTaskで明示されたファイルだけを含めます。コミットメッセージは英語の通常文とし、Conventional Commits形式を使いません。

---

## File Map

`Cargo.toml`は依存関係とバイナリ定義、`src/main.rs`は終了コードへの変換だけを担当します。`src/cli.rs`はclapの入力型、`src/output.rs`はhuman/JSON表示を担当します。

`src/domain.rs`はIssue状態、優先度、実行コンテキスト、IssueとProjectの公開型を持ちます。`src/error.rs`はアプリケーションエラーと安定した終了コードを定義します。`src/app.rs`はCLIから呼ばれるユースケースを実装します。

`src/store/mod.rs`はStoreの公開入口、`src/store/sqlite.rs`は接続設定とトランザクション、`src/store/schema.sql`は初期スキーマを担当します。Phase 1では交換可能なストレージtraitを作らず、現在必要なSQLite実装だけを置きます。

`tests/cli_*.rs`はユーザーから見えるコマンド契約、`tests/support/mod.rs`は一時DBとコマンド起動を共有します。`README.md`は最短利用手順、`docs/json-contract.md`は機械向け出力契約を記載します。

### Task 1: Rust CLI Skeleton and Stable Errors

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/cli.rs`
- Create: `src/error.rs`
- Create: `src/output.rs`
- Create: `tests/cli_help.rs`

**Interfaces:**
- Produces: `cli::Cli::parse()`、`error::AppError`、`error::ExitCode`、`output::OutputMode`

- [ ] **Step 1: Verify the managed Rust runtime**

Run: `command -v rustc && rustc --version && mise exec -- cargo --version`

Expected: `rustc`と`cargo`の解決先とバージョンが表示され、Rust 2024 editionを利用できます。

- [ ] **Step 2: Write the failing CLI help test**

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_names_the_product_and_core_commands() {
    Command::cargo_bin("bettr")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Local issue tracking for agent work"))
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("project"))
        .stdout(predicate::str::contains("issue"))
        .stdout(predicate::str::contains("status"));
}
```

- [ ] **Step 3: Run the test and verify the binary is absent**

Run: `mise exec -- cargo test --test cli_help`

Expected: FAIL because `Cargo.toml` or the `bettr` binary does not exist.

- [ ] **Step 4: Create the package and CLI types**

Define `Cli { database: Option<PathBuf>, project: Option<String>, json: bool, command: Command }`. Define the initial `Command` variants as `Init`, `Project`, `Issue`, and `Status`; nested commands may initially carry no behavior. Define `OutputMode::Human | Json` and these stable exit codes:

```rust
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    InvalidInput = 2,
    NotFound = 3,
    Conflict = 4,
    DatabaseBusy = 5,
    Internal = 10,
}
```

Use clap's derive API and the package description `Local issue tracking for agent work`. In `main`, parse the CLI and return success for help parsing; unimplemented command execution may return `AppError::Internal("command is not implemented")`.

- [ ] **Step 5: Run formatting, lint, and the focused test**

Run: `mise exec -- cargo fmt --check`

Run: `mise exec -- cargo clippy --all-targets -- -D warnings`

Run: `mise exec -- cargo test --test cli_help`

Expected: all commands succeed.

- [ ] **Step 6: Commit the skeleton**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/cli.rs src/error.rs src/output.rs tests/cli_help.rs
git commit -m "Initialize the bettr CLI"
```

### Task 2: SQLite Initialization and Connection Policy

**Files:**
- Create: `src/store/mod.rs`
- Create: `src/store/sqlite.rs`
- Create: `src/store/schema.sql`
- Create: `tests/support/mod.rs`
- Create: `tests/cli_init.rs`
- Modify: `src/main.rs`
- Modify: `src/cli.rs`

**Interfaces:**
- Consumes: `AppError`, `ExitCode`
- Produces: `store::Database::initialize(path: &Path)`、`store::Database::open(path: &Path)`、`store::Database::connection()`

- [ ] **Step 1: Write initialization contract tests**

Create tests that run `bettr --database <temp>/bettr.db init --json`, assert exit 0 and this response, then run `init` again and assert exit 2 without replacing the file:

```json
{"schema_version":1,"data":{"initialized":true}}
```

Add a test that runs `project list` against a missing database and asserts exit 3 with error code `database_not_initialized` on stderr.

- [ ] **Step 2: Run the initialization tests**

Run: `mise exec -- cargo test --test cli_init`

Expected: FAIL because initialization and the schema are absent.

- [ ] **Step 3: Define the initial schema**

Create tables `projects`, `issues`, `comments`, `domain_events`, and `audit_events`. Use TEXT UUID primary keys, INTEGER project-local issue numbers, UTC TEXT timestamps, INTEGER revisions, foreign keys, and `UNIQUE(project_id, number)`. Store the five states using a CHECK constraint. Add indexes for `issues(project_id, state, updated_at)` and event sequence lookup. Set `PRAGMA user_version = 1` in the initialization transaction.

- [ ] **Step 4: Implement connection setup and explicit initialization**

`Database::open` must fail if the file does not exist. For every connection execute `PRAGMA foreign_keys = ON`, `PRAGMA journal_mode = WAL`, and set a five-second busy timeout. `Database::initialize` must create the parent directory with owner-only permissions, open with create-new semantics, apply `schema.sql` in one immediate transaction, and remove the new file if schema application fails.

- [ ] **Step 5: Implement the shared CLI test harness**

Define `TestApp { dir: TempDir, database: PathBuf }` with `command()` returning an `assert_cmd::Command` preconfigured with `--database`. Keep environment isolation inside this helper so every CLI test uses a fresh database.

- [ ] **Step 6: Run focused and full verification**

Run: `mise exec -- cargo test --test cli_init`

Run: `mise exec -- cargo test`

Expected: all tests pass.

- [ ] **Step 7: Commit database initialization**

```bash
git add src/main.rs src/cli.rs src/store tests/support tests/cli_init.rs
git commit -m "Add explicit SQLite initialization"
```

### Task 3: Projects and Execution Context

**Files:**
- Create: `src/domain.rs`
- Create: `src/app.rs`
- Create: `tests/cli_project.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/store/sqlite.rs`

**Interfaces:**
- Produces: `Project { id: Uuid, name: String, archived: bool, created_at: DateTime<Utc> }`
- Produces: `ExecutionContext { kind: InitiatorKind, agent: Option<String>, session_id: Option<String>, operator: Option<String> }`
- Produces: `App::create_project(name, context)`、`App::list_projects()`

- [ ] **Step 1: Write project lifecycle tests**

Test that `project create bettr --json` returns UUID, name, `archived:false`, and UTC timestamp. Test that duplicate names fail with exit 4 and `project_name_conflict`. Test `project list --json` and verify deterministic name ordering. Set `BETTR_AGENT=codex` and `BETTR_SESSION_ID=session-1`, then assert an audit row records `kind=agent`, agent, session, operation, success, and project UUID without recording raw argv.

- [ ] **Step 2: Run the project tests**

Run: `mise exec -- cargo test --test cli_project`

Expected: FAIL because project commands and domain types are absent.

- [ ] **Step 3: Implement domain parsing and execution context resolution**

Define `InitiatorKind::Agent | Human | System`. If `BETTR_AGENT` exists, resolve agent context and optional `BETTR_SESSION_ID`. Otherwise resolve human context using `BETTR_OPERATOR`, falling back to the OS username. Reject empty names and values longer than 200 Unicode scalar values.

- [ ] **Step 4: Implement project commands and transactional audit writes**

Create `project create` and `project list`. Every command must call an application wrapper that writes an audit start/result record. A write operation must create its domain event and successful audit result in the same transaction as the project change. For a failed operation, write a failure audit event in a separate short transaction after rollback.

- [ ] **Step 5: Run project and regression tests**

Run: `mise exec -- cargo test --test cli_project`

Run: `mise exec -- cargo test`

Expected: all tests pass.

- [ ] **Step 6: Commit project support**

```bash
git add src/main.rs src/cli.rs src/domain.rs src/app.rs src/store/sqlite.rs tests/cli_project.rs
git commit -m "Add project management and execution context"
```

### Task 4: Issue Creation, Display, and Project Resolution

**Files:**
- Create: `tests/cli_issue_create.rs`
- Modify: `src/domain.rs`
- Modify: `src/app.rs`
- Modify: `src/cli.rs`
- Modify: `src/output.rs`
- Modify: `src/store/sqlite.rs`

**Interfaces:**
- Produces: `IssueState`、`Priority`、`Issue`、`NewIssue`
- Produces: `App::create_issue(project, input, context)`、`App::show_issue(project, number)`

- [ ] **Step 1: Write Issue creation and display tests**

Test `issue create --project bettr --title "Build local core" --body "First vertical slice" --priority high --json`. Assert project-local number 1, UUID, state `todo`, revision 1, timestamps, title, body, priority, and no assignee. Create a second Issue and assert number 2. Test `issue show 1 --project bettr --json`, unknown Issue exit 3, missing project exit 2, and unknown project exit 3.

- [ ] **Step 2: Run the Issue creation tests**

Run: `mise exec -- cargo test --test cli_issue_create`

Expected: FAIL because Issue operations are absent.

- [ ] **Step 3: Define Issue input and output types**

Use exhaustive enums for the five states and four priorities. Define `Issue` with `id`, `project_id`, `number`, `title`, optional `body`, state, optional priority, optional `assignee_kind`, optional `assignee_name`, `revision`, `created_at`, and `updated_at`. Validate nonblank titles up to 500 Unicode scalar values and bodies up to 1 MiB.

- [ ] **Step 4: Allocate project-local numbers atomically**

Inside an immediate transaction, read `MAX(number) + 1` for the project, insert the Issue, append `issue_created` to `domain_events`, and append the successful audit result. Retry only SQLite busy errors; rely on the unique constraint to reject an impossible duplicate allocation.

- [ ] **Step 5: Render stable JSON envelopes**

Every success response must use `{"schema_version":1,"data":...}`. Every error written to stderr in JSON mode must use `{"schema_version":1,"error":{"code":"...","message":"..."}}`. Human `show` output must include `project#number`, state, title, revision, priority, assignee, and body without terminal escape interpretation.

- [ ] **Step 6: Run focused and full tests**

Run: `mise exec -- cargo test --test cli_issue_create`

Run: `mise exec -- cargo test`

Expected: all tests pass.

- [ ] **Step 7: Commit Issue creation**

```bash
git add src/domain.rs src/app.rs src/cli.rs src/output.rs src/store/sqlite.rs tests/cli_issue_create.rs
git commit -m "Add Issue creation and display"
```

### Task 5: Listing, Filtering, Search, and Status

**Files:**
- Create: `tests/cli_issue_list.rs`
- Create: `tests/cli_status.rs`
- Modify: `src/app.rs`
- Modify: `src/cli.rs`
- Modify: `src/output.rs`
- Modify: `src/store/schema.sql`
- Modify: `src/store/sqlite.rs`

**Interfaces:**
- Produces: `IssueFilter { projects, states, priorities, assignee, updated_after, query, include_done }`
- Produces: `App::list_issues(filter)`、`App::status()`

- [ ] **Step 1: Write list filtering tests**

Seed Issues across two projects. Verify default `issue list` requires a resolved project and excludes `done` and `cancelled`. Verify `--all-projects`, repeated `--state`, `--priority`, `--assignee`, `--updated-after`, `--include-completed`, and title/body query filters. Assert deterministic ordering by attention placeholder, blocked state, in-progress state, priority, then creation time.

- [ ] **Step 2: Write status tests**

Verify `status --json` crosses all projects and groups blocked, recently completed, and active Issues. Phase 1 has no decision requests or leases, so response fields `attention` and `stale` must exist as empty arrays to keep the future JSON shape additive.

- [ ] **Step 3: Run list and status tests**

Run: `mise exec -- cargo test --test cli_issue_list --test cli_status`

Expected: FAIL because filtering and status are absent.

- [ ] **Step 4: Add Phase 1 search indexes and queries**

Add normalized title/body columns or an FTS5 virtual table only if bundled SQLite reports FTS5 support in a focused capability test. Otherwise use escaped `LIKE` with indexes on state, priority, assignee, and updated time. Do not introduce a search abstraction; expose one `list_issues` query builder scoped to the defined filters.

- [ ] **Step 5: Implement human and JSON status rendering**

Human output must group sections in this order: attention, stale, blocked, recently completed, active. Empty sections are omitted in human output but retained as empty arrays in JSON. Use project-qualified Issue references in cross-project output.

- [ ] **Step 6: Run focused and full tests**

Run: `mise exec -- cargo test --test cli_issue_list --test cli_status`

Run: `mise exec -- cargo test`

Expected: all tests pass.

- [ ] **Step 7: Commit read views**

```bash
git add src/app.rs src/cli.rs src/output.rs src/store/schema.sql src/store/sqlite.rs tests/cli_issue_list.rs tests/cli_status.rs
git commit -m "Add Issue filters and supervisor status"
```

### Task 6: State Transitions and Optimistic Locking

**Files:**
- Create: `tests/cli_issue_transition.rs`
- Modify: `src/domain.rs`
- Modify: `src/app.rs`
- Modify: `src/cli.rs`
- Modify: `src/store/sqlite.rs`

**Interfaces:**
- Produces: `Transition::{Start, Block, Resume, Complete, Cancel, Reopen}`
- Produces: `App::transition_issue(project, number, expected_revision, transition, context)`

- [ ] **Step 1: Write transition table tests**

Test every allowed edge and representative rejected edges. Require `--revision` on all transition commands. Require `--reason` and `--wait-kind` for block, `--summary` and `--verification` for complete, and `--reason` for cancel and reopen. Assert each success increments revision exactly once and appends one domain event.

- [ ] **Step 2: Write stale revision and concurrent update tests**

Read revision 1 twice, update once, then attempt a second update with revision 1. Assert exit 4, error code `revision_conflict`, current revision 2 in error details, and unchanged Issue data. Run two child processes against the same database and assert only one identical revision update succeeds.

- [ ] **Step 3: Run transition tests**

Run: `mise exec -- cargo test --test cli_issue_transition`

Expected: FAIL because transition commands are absent.

- [ ] **Step 4: Implement transition validation as a pure domain function**

Define `IssueState::apply(&self, transition: &Transition) -> Result<IssueState, DomainError>`. Keep required metadata validation in constructors such as `Transition::complete(summary, verification)`. Do not let SQLite rows or clap arguments construct invalid transition values directly.

- [ ] **Step 5: Implement compare-and-swap updates**

Use `UPDATE issues SET ..., revision = revision + 1 WHERE id = ? AND revision = ?`. If affected rows are zero, query current revision to distinguish missing Issue from conflict. Insert the domain event and audit result only after the compare-and-swap succeeds, within the same transaction.

- [ ] **Step 6: Run transition, concurrency, and regression tests**

Run: `mise exec -- cargo test --test cli_issue_transition -- --test-threads=1`

Run: `mise exec -- cargo test`

Expected: all tests pass.

- [ ] **Step 7: Commit transitions**

```bash
git add src/domain.rs src/app.rs src/cli.rs src/store/sqlite.rs tests/cli_issue_transition.rs
git commit -m "Add guarded Issue state transitions"
```

### Task 7: Issue Editing, Assignment, Comments, and History

**Files:**
- Create: `tests/cli_issue_edit.rs`
- Create: `tests/cli_comment_history.rs`
- Modify: `src/domain.rs`
- Modify: `src/app.rs`
- Modify: `src/cli.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/output.rs`

**Interfaces:**
- Produces: `IssuePatch`、`AssigneeKind::Human | Agent`
- Produces: `Comment`、`DomainEvent`
- Produces: `App::update_issue`、`App::add_comment`、`App::issue_history`

- [ ] **Step 1: Write Issue edit and assignment tests**

Test title, body, priority, `assignee_kind`, and `assignee_name` changes with a required revision. Reject an assignee name without kind and a kind without name. Assert a patch that changes no fields exits 2. Assert revision conflict behavior matches state transitions.

- [ ] **Step 2: Write comment and history tests**

Add two comments and assert immutable IDs, UTC timestamps, execution context, and insertion order. Verify there is no edit or delete comment command. Verify `issue history` returns Issue creation, edits, comments, and transitions in event sequence order without exposing audit-only reads.

- [ ] **Step 3: Run edit and comment tests**

Run: `mise exec -- cargo test --test cli_issue_edit --test cli_comment_history`

Expected: FAIL because these operations are absent.

- [ ] **Step 4: Implement revision-guarded patches and immutable comments**

Represent omitted patch fields distinctly from explicit clearing. Permit clearing body, priority, and assignee but not title. Insert comments without updating Issue revision; update `issues.updated_at` so activity ordering changes, and append a `comment_added` domain event in the same transaction.

- [ ] **Step 5: Implement domain history projection**

Return event sequence, event type, Issue revision when applicable, timestamp, execution context, and safe structured metadata. Comment events may return comment ID and body in Issue history because history is domain data; audit events must continue to omit content.

- [ ] **Step 6: Run focused and full tests**

Run: `mise exec -- cargo test --test cli_issue_edit --test cli_comment_history`

Run: `mise exec -- cargo test`

Expected: all tests pass.

- [ ] **Step 7: Commit Issue editing and history**

```bash
git add src/domain.rs src/app.rs src/cli.rs src/store/sqlite.rs src/output.rs tests/cli_issue_edit.rs tests/cli_comment_history.rs
git commit -m "Add Issue editing comments and history"
```

### Task 8: Audit Queries, Context Inspection, and Failure Recording

**Files:**
- Create: `tests/cli_audit.rs`
- Create: `tests/cli_context.rs`
- Modify: `src/app.rs`
- Modify: `src/cli.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/output.rs`

**Interfaces:**
- Produces: `AuditEvent`、`ResolvedContext`
- Produces: `App::list_audit_events(filter)`、`App::resolved_context()`

- [ ] **Step 1: Write complete audit coverage tests**

Run a successful read, successful write, invalid transition, missing Issue lookup, revision conflict, and malformed input that reaches application execution. Assert audit events include operation, target IDs when known, project UUID and operation-time project name, execution context, started/finished timestamps, outcome, exit code, and resulting revision. Assert no event contains title, body, comment, raw argv, or environment values unrelated to identity.

- [ ] **Step 2: Write context precedence tests**

Test project resolution precedence as CLI argument, environment variable, directory configuration, user configuration, default. Test database precedence as CLI argument, `BETTR_DATABASE`, user configuration, OS default. Verify `bettr context --json` returns each resolved value and its source without creating a database.

- [ ] **Step 3: Run audit and context tests**

Run: `mise exec -- cargo test --test cli_audit --test cli_context`

Expected: FAIL because audit queries and context inspection are incomplete.

- [ ] **Step 4: Implement safe audit payloads and audit listing**

Define an allowlist per operation rather than removing sensitive keys after serialization. Support filters for project UUID, operation, outcome, execution kind, agent, session ID, and timestamp. Human output must be concise; JSON must expose the full safe event.

- [ ] **Step 5: Implement configuration files without implicit mutation**

Read user config from the platform config directory and directory config from `.bettr.toml` while walking from the current directory to the filesystem root. Phase 1 only reads these files; add no config-writing command. Reject ambiguous or invalid config with exit 2.

- [ ] **Step 6: Run focused and full tests**

Run: `mise exec -- cargo test --test cli_audit --test cli_context`

Run: `mise exec -- cargo test`

Expected: all tests pass.

- [ ] **Step 7: Commit audit visibility and context resolution**

```bash
git add src/app.rs src/cli.rs src/store/sqlite.rs src/output.rs tests/cli_audit.rs tests/cli_context.rs
git commit -m "Add audit queries and context inspection"
```

### Task 9: SQLite Contention and Integrity Tests

**Files:**
- Create: `tests/sqlite_concurrency.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/error.rs`

**Interfaces:**
- Consumes: `Database` and all Phase 1 write operations
- Produces: stable mapping from SQLite busy/locked conditions to `AppError::DatabaseBusy`

- [ ] **Step 1: Write contention tests with independent processes**

Hold an immediate write transaction from a test helper process, invoke a second `bettr` write, release before five seconds, and assert the second write succeeds. Repeat while holding longer than five seconds and assert exit 5 with `database_busy`. Verify reads continue while the WAL writer is open.

- [ ] **Step 2: Write integrity tests after mixed concurrent operations**

Launch multiple processes that create Issues and comments in the same project. Assert unique contiguous Issue numbers, valid foreign keys, successful `PRAGMA integrity_check`, and one successful audit event per committed operation.

- [ ] **Step 3: Run contention tests before changing store behavior**

Run: `mise exec -- cargo test --test sqlite_concurrency -- --test-threads=1`

Expected: at least the long-lock error mapping or retry timing test fails.

- [ ] **Step 4: Centralize SQLite error classification**

Add one conversion function that maps `SQLITE_BUSY`, `SQLITE_BUSY_SNAPSHOT`, and `SQLITE_LOCKED` to `DatabaseBusy`; constraint failures to domain-specific conflict or invalid-input errors at their call sites; and all remaining SQLite errors to `Internal` with safe diagnostics.

- [ ] **Step 5: Run concurrency and full verification**

Run: `mise exec -- cargo test --test sqlite_concurrency -- --test-threads=1`

Run: `mise exec -- cargo test`

Run: `mise exec -- cargo clippy --all-targets -- -D warnings`

Expected: all commands pass without flaky timing retries.

- [ ] **Step 6: Commit concurrency hardening**

```bash
git add src/store/sqlite.rs src/error.rs tests/sqlite_concurrency.rs
git commit -m "Harden SQLite concurrency behavior"
```

### Task 10: Documentation, Performance Baseline, and Phase 1 Acceptance

**Files:**
- Create: `README.md`
- Create: `LICENSE`
- Create: `docs/json-contract.md`
- Create: `benches/cli_latency.rs`
- Create: `tests/phase1_workflow.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: all Phase 1 commands and JSON types
- Produces: documented Phase 1 user workflow and repeatable latency baseline

- [ ] **Step 1: Write the end-to-end acceptance test**

The test must initialize a database, create a project, create an Issue, assign and start it, add a comment, block it, resume it, complete it, inspect history, inspect status, and query audit events. Parse every JSON response with serde_json and assert revisions, states, event order, and execution context rather than matching presentation strings.

- [ ] **Step 2: Run the acceptance test**

Run: `mise exec -- cargo test --test phase1_workflow`

Expected: PASS if Tasks 1 through 9 satisfy the complete workflow; otherwise fix only the exposed contract gap before continuing.

- [ ] **Step 3: Document installation and the shortest workflow**

README must show `cargo install --path .`, `bettr init`, project creation, Issue creation, start, block, resume, complete, `status`, JSON mode, environment-based execution context, and the macOS/Linux data paths. State explicitly that the Phase 1 CLI does not start agents, provide network sharing, or support external databases.

- [ ] **Step 4: Document the JSON and exit-code contract**

`docs/json-contract.md` must define the success envelope, error envelope, additive field policy, `schema_version: 1`, UTC timestamp representation, Issue reference format, and exit codes 0, 2, 3, 4, 5, and 10 with concrete examples generated from acceptance fixtures.

- [ ] **Step 5: Add the MIT license**

Use the standard MIT License text with copyright holder `asonas` and year `2026`.

- [ ] **Step 6: Add a reproducible latency baseline**

Create a release-mode harness that prepares a fixed database, runs 1,000 `issue show` operations and 1,000 revision-guarded updates as child processes, sorts durations, and reports p50 and p95. Do not assert the 50 ms target in normal unit tests; document machine information and results in release notes when publishing.

- [ ] **Step 7: Run the Phase 1 release gate**

Run: `mise exec -- cargo fmt --check`

Run: `mise exec -- cargo clippy --all-targets -- -D warnings`

Run: `mise exec -- cargo test`

Run: `mise exec -- cargo build --release`

Run: `mise exec -- cargo bench --bench cli_latency`

Expected: formatting, lint, tests, and release build pass; benchmark output includes p50 and p95 for reads and writes.

- [ ] **Step 8: Commit Phase 1 documentation and acceptance**

```bash
git add README.md LICENSE docs/json-contract.md benches/cli_latency.rs tests/phase1_workflow.rs Cargo.toml Cargo.lock
git commit -m "Document and verify the Phase 1 workflow"
```

## Self-Review Record

The plan covers every Phase 1 requirement from `docs/design.md`: explicit initialization, projects, five Issue states, comments, revisions, human and JSON output, cross-project status, execution context, SQLite audit events, concurrency, stable errors, documentation, MIT licensing, and a performance baseline. Agent claim and lease, decision requests, dependencies, parent-child Issues, references, event cursors, idempotency, JSONL audit, backup, restore, redaction, doctor, skills packaging, and GitHub Releases remain intentionally assigned to later roadmap phases.
