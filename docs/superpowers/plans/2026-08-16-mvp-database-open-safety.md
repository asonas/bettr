# MVP Database Open Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 誤って非bettr SQLiteを指定しても通常のCLI操作で変更せず、MVPの非敵対的なローカル利用に必要なDB identity保護を完成させます。

**Architecture:** SQLite接続より前にファイルheaderから`application_id`と`user_version`を読み、非bettr DBを副作用なしで拒否します。合格後にread-write接続を開き、変更系PRAGMAより前に同じ接続でidentityを再確認します。二つの確認間に第三者が意図的にファイルを置換する攻撃はMVPの保証外として明文化します。

**Tech Stack:** Rust 2024 edition、rusqlite bundled、既存のassert_cmd/tempfile統合テスト

**Spec:** `docs/design.md`

## Global Constraints

Rustコマンドの前に`command -v rustc`と`rustc --version`を確認し、Cargoは`mise exec -- cargo ...`で実行します。

通常の誤指定から非bettr DBを保護しますが、identity確認とopenの間に別プロセスが意図的にパスを置換するTOCTOU攻撃はMVPの保証外です。

非bettr DBの判定中にSQLite接続を開かず、WAL、SHM、journal、schema、data、元ファイルbytesを変更しません。

identity一致後のread-write接続では、WALなどの変更系PRAGMAを実行する前に同じ接続でidentityを再確認します。

既存の終了契約を維持し、非bettr DBはexit 3の`database_not_initialized`として扱います。互換layerやschema migrationは追加しません。

---

### Task 1: Side-Effect-Free Identity Preflight

**Files:**
- Modify: `src/store/sqlite.rs`
- Test: `tests/cli_init.rs`

**Interfaces:**
- Produces: `DatabaseIdentity { application_id: u32, user_version: u32 }`
- Produces: `read_sqlite_header_identity(path: &Path) -> Result<DatabaseIdentity, AppError>`
- Consumes: existing `BETTR_APPLICATION_ID` and schema version 1 constants

- [ ] **Step 1: Write the failing non-bettr WAL safety test**

Create a valid unrelated SQLite DB, switch it to WAL, checkpoint it, close it, remove any pre-existing sidecars, and snapshot its bytes, schema, sentinel data, journal mode, and directory entries. Run `bettr --database <path> project list --json`. Assert exit 3 with `database_not_initialized`, unchanged bytes/schema/data/journal mode, and no newly created `-wal`, `-shm`, or `-journal` files.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `mise exec -- cargo test --test cli_init project_list_does_not_touch_a_non_bettr_wal_database -- --exact`

Expected: FAIL because the current identity preflight opens SQLite before rejection and can create sidecars or otherwise touch WAL state.

- [ ] **Step 3: Implement raw SQLite-header identity parsing**

Read exactly the first 100 bytes with `std::fs::File`. Validate the SQLite header magic `SQLite format 3\0`. Parse `user_version` from bytes 60..64 and `application_id` from bytes 68..72 as big-endian `u32`. Short files, non-SQLite files, wrong application ID, and wrong schema version map to `DatabaseNotInitialized` without exposing file contents.

```rust
struct DatabaseIdentity {
    application_id: u32,
    user_version: u32,
}

impl DatabaseIdentity {
    fn is_current_bettr(&self) -> bool {
        self.application_id == BETTR_APPLICATION_ID && self.user_version == 1
    }
}
```

- [ ] **Step 4: Recheck identity on the opened connection before configuration**

Change `Database::open` to perform raw-header preflight first. Then open with read-write/no-create flags, query `PRAGMA application_id` and `PRAGMA user_version` on that same connection, and reject a mismatch before calling the function that sets busy timeout, foreign keys, or WAL. Keep initialization on its dedicated create-new path.

- [ ] **Step 5: Add boundary tests for malformed and bettr files**

Add focused tests for a short file, a non-SQLite file, a SQLite DB with the wrong application ID, and a valid bettr DB. The first three must return exit 3 without changing bytes or creating sidecars; the valid DB must continue through the normal command path.

- [ ] **Step 6: Run focused and regression verification**

Run: `mise exec -- cargo test --test cli_init`

Run: `mise exec -- cargo test --test sqlite_concurrency production_database_connections_enable_integrity_and_contention_pragmas -- --exact`

Run: `mise exec -- cargo test`

Run: `mise exec -- cargo clippy --all-targets -- -D warnings`

Expected: all commands succeed with pristine output.

- [ ] **Step 7: Commit the identity preflight**

```bash
git add src/store/sqlite.rs tests/cli_init.rs
git commit -m "Protect non-bettr databases during open"
```

### Task 2: MVP Safety Contract and Release Gate

**Files:**
- Modify: `docs/design.md`
- Modify: `README.md`
- Modify: `docs/json-contract.md`
- Modify: `tests/phase1_workflow.rs`

**Interfaces:**
- Consumes: Task 1's `database_not_initialized` behavior
- Produces: documented MVP threat model and final acceptance evidence

- [ ] **Step 1: Add an acceptance test for the documented error contract**

Extend the Phase 1 acceptance test or add one focused acceptance case that passes an unrelated SQLite path and asserts exit 3, JSON error code `database_not_initialized`, unchanged file bytes, and absence of new sidecars.

- [ ] **Step 2: Run the acceptance test before documentation changes**

Run: `mise exec -- cargo test --test phase1_workflow`

Expected: PASS against Task 1 behavior. This step characterizes the contract that the documentation will publish; it does not require production changes.

- [ ] **Step 3: Document the MVP threat model**

Update the design and README to state that bettr protects unrelated databases from accidental path selection by using a side-effect-free header preflight and same-connection recheck. State explicitly that an adversarial process replacing the path between checks is outside the MVP guarantee. Keep the JSON contract's exit 3 example aligned with `database_not_initialized`.

- [ ] **Step 4: Run the final release gate**

Run: `command -v rustc && rustc --version`

Run: `mise exec -- cargo fmt --check`

Run: `mise exec -- cargo clippy --all-targets -- -D warnings`

Run: `mise exec -- cargo test`

Run: `mise exec -- cargo build --release`

Run: `mise exec -- cargo bench --bench cli_latency`

Expected: formatting, lint, all tests, release build, and the three latency series succeed. Record the test count and p50/p95 output in the implementation report.

- [ ] **Step 5: Commit the MVP safety contract**

```bash
git add docs/design.md README.md docs/json-contract.md tests/phase1_workflow.rs
git commit -m "Document the MVP database safety boundary"
```

### Fix Round: Final Review

- [x] 別用途のSQLite pathに対するcommand別契約をPhase 1 acceptanceで固定する。
- [x] `init --json`はexit 2の`database_already_initialized`、`context --json`はexit 0、初期化済みDBを必要とする`project list --json`はexit 3の`database_not_initialized`とする。
- [x] Unix FIFOをdatabase pathに指定してもwriter待ちでblockせず、exit 3のJSON errorを返す回帰testをREDからGREENにする。
- [x] raw headerをopenする前に、symlinkの参照先を追跡するmetadataで通常ファイルか確認する。
- [x] symlink経由の有効なbettr DBは引き続き受理する。
- [x] README、設計、JSON契約のexit 3説明を、初期化済みDBを必要とするcommandへ限定する。
- [x] `init`と`context`の例外契約を明記する。
- [x] rustfmt、Clippy、全103 tests、release build、3系列のCLI latency benchmarkをfresh実行する。

## Self-Review Record

This plan resolves the only load-bearing final-review finding while honoring the user's explicit MVP tradeoff. It does not claim adversarial TOCTOU resistance, introduce OS-specific locking, add a daemon, change schema version 1, or broaden Phase 1 into Phase 2 or Phase 3.
