# SQLiteスキーマ移行基盤 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 既存のschema version 1データベースをversion 2へ安全に移行し、将来のSQLiteスキーマ変更を履歴・トランザクション・監査付きで適用できる基盤を追加する。

**Architecture:** Migration定義とトランザクション内の適用処理を`src/store/migrations.rs`へ分離し、`Database::open`が接続後に未適用Migrationを実行する。新規初期化は最新schema version 2を作成し、既存version 1は`BEGIN IMMEDIATE`で`schema_migrations`を作成してversion 2へ進める。未知versionはSQLiteをread-writeで開く前に拒否する。

**Tech Stack:** Rust 2024、rusqlite 0.38 bundled SQLite、assert_cmd、serde_json、tempfile

**Spec:** `docs/superpowers/specs/2026-08-16-schema-migration-foundation-design.md`

## Global Constraints

- JSONレスポンスの`schema_version: 1`は変更しない。
- SQLiteの最新schema versionは2、bettrのapplication IDは既存値`0x4254_5452`を維持する。
- schema version 1の既存DBだけを移行元として受け入れ、未知versionはSQLite接続・WAL設定・監査書き込みなしで拒否する。
- MigrationのDDL/DML、履歴行、`PRAGMA user_version`、成功監査イベントは同じ`BEGIN IMMEDIATE`トランザクションへ含める。
- Migration履歴にはversion、固定name、UTCのapplied_atだけを保存し、checksum、dirty状態、外部設定、CLIを追加しない。
- Rustの同一crate内参照は`crate::`を使い、不要なグローバル`use`を追加しない。
- 各振る舞いは、実装前に失敗するテストを作成してから最小実装を追加する。

---

### Task 1: 移行契約とエラー契約の失敗テストを追加

**Files:**
- Create: `src/store/migrations.rs`（Migration実行器の失敗テストだけを先に追加）
- Modify: `src/store/mod.rs`（`migrations`モジュールを登録）
- Modify: `tests/cli_init.rs`
- Test: `src/store/migrations.rs`、`tests/cli_init.rs`

**Interfaces:**
- Produces: テストが参照する`crate::store::migrations::Migration`、`apply_pending`の期待インターフェースと、version 2初期化・version 1移行・未知version拒否の受け入れ条件。

- [ ] **Step 1: Migration実行器のrollbackテストを書く**

`src/store/migrations.rs`に、まだ実装されていない次のAPIを参照するテストを追加する。Migration関数はテーブルを作成してからSQLiteエラーを返し、呼び出し側がトランザクションをdropした後に作成物と`user_version`が残らないことを検証する。

```rust
#[cfg(test)]
mod tests {
    fn failing_migration(
        transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), rusqlite::Error> {
        transaction.execute_batch(
            "CREATE TABLE rollback_target (value TEXT NOT NULL);\n\
             INSERT INTO rollback_target VALUES ('must disappear');",
        )?;
        Err(rusqlite::Error::InvalidQuery)
    }

    #[test]
    fn failed_migration_can_be_rolled_back_without_history_or_version_change() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        let migration = super::Migration {
            version: 2,
            name: "failing migration",
            apply: failing_migration,
        };

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            assert!(super::apply_pending(&transaction, &[migration]).is_err());
        }

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'rollback_target'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 0);
    }
}
```

- [ ] **Step 2: 初期化後のschema versionと履歴を検証するテストへ変更する**

`tests/cli_init.rs`の`init_creates_a_version_one_database_once`を`init_creates_a_version_two_database_with_migration_history`へ変更し、`user_version`が2であることと、次の2行が`schema_migrations`に存在することを追加で検証する。

```rust
let migrations = connection
    .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
    .unwrap()
    .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
assert_eq!(
    migrations,
    vec![(1, "phase1_baseline".to_owned()), (2, "schema_migrations".to_owned())]
);
```

- [ ] **Step 3: schema version 1から2への移行テストを書く**

初期化とプロジェクト作成後、テスト用接続で`DROP TABLE IF EXISTS schema_migrations; PRAGMA user_version = 1;`を実行してPhase 1 DBを再現する。`project list --json`後にversion 2、履歴2行、既存project、`schema_migrate`監査1件を検証する。Migration metadataはJSONとして読み、`from_version: 1`、`to_version: 2`、`migration: "schema_migrations"`だけを確認する。

- [ ] **Step 4: 未知version拒否のテストを書く**

初期化済みbettr DBの`PRAGMA user_version`を99へ変更してbytesを保存し、`project list --json`が終了コード2、エラーコード`unsupported_database_schema_version`、detailsの`found_version: 99`と`current_version: 2`を返すことを確認する。元bytesが不変で、`-wal`、`-shm`、`-journal`が新規作成されないことも確認する。

- [ ] **Step 5: 失敗テストを実行する**

Run: `mise exec -- cargo test --test cli_init --test phase1_workflow`

Expected: 新しいversion 2・履歴・移行・未知versionのアサーションと、未実装の`Migration`/`apply_pending`参照が原因でFAILする。既存のPhase 1コードが通ることではなく、要求した振る舞いが未実装であることを確認する。

### Task 2: Migration定義とトランザクション内実行器を実装

**Files:**
- Modify: `src/store/migrations.rs`
- Modify: `src/store/mod.rs`
- Modify: `src/store/schema.sql`
- Test: `src/store/migrations.rs`

**Interfaces:**
- Consumes: Task 1の`Migration`/`apply_pending`テスト。
- Produces: `pub(crate) const LATEST_SCHEMA_VERSION: u32 = 2`、`pub(crate) const BASE_SCHEMA_VERSION: u32 = 1`、`pub(crate) fn is_supported_version(version: u32) -> bool`、`pub(crate) fn apply_pending(transaction: &rusqlite::Transaction<'_>, migrations: &[Migration]) -> Result<Vec<Migration>, rusqlite::Error>`。

- [ ] **Step 1: Migration型と実行器の最小実装を書く**

`Migration`は`version: u32`、`name: &'static str`、`apply: fn(&rusqlite::Transaction<'_>) -> Result<(), rusqlite::Error>`を持つ`Copy`型にする。`apply_pending`はTransaction内で`PRAGMA user_version`を読み、ロック待機後の最新versionより大きいMigrationだけを登録順に適用する。成功ごとに履歴行と`PRAGMA user_version`を同じTransactionへ書き込む。失敗時はエラーを返し、Transactionの所有者がdropしてrollbackできるように内部でcommitしない。

```rust
#[derive(Clone, Copy)]
pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) name: &'static str,
    pub(crate) apply: fn(&rusqlite::Transaction<'_>) -> Result<(), rusqlite::Error>,
}

pub(crate) fn apply_pending(
    transaction: &rusqlite::Transaction<'_>,
    migrations: &[Migration],
) -> Result<Vec<Migration>, rusqlite::Error> {
    let current_version: i64 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))?;
    let mut applied = Vec::new();
    for migration in migrations
        .iter()
        .filter(|migration| i64::from(migration.version) > current_version)
    {
        (migration.apply)(transaction)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![
                migration.version,
                migration.name,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        transaction.pragma_update(None, "user_version", migration.version)?;
        applied.push(*migration);
    }
    Ok(applied)
}
```

- [ ] **Step 2: version検証を実装する**

version 1以上かつversion 2以下だけをsupportedとする`is_supported_version`を追加する。未知versionを`AppError`へ変換する処理は、JSONエラー契約と同時にTask 3で実装する。

- [ ] **Step 3: version 2 Migrationを実装する**

固定Migration配列にversion 2、name `schema_migrations`を登録する。Migration関数は次のDDLを実行し、version 1のbaseline行を`INSERT OR IGNORE`で追加する。

```sql
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
);
```

実行器がversion 2行を追加するため、Migration関数自身はversion 2の履歴行を追加しない。

- [ ] **Step 4: 初期schemaをversion 2へ更新する**

`schema.sql`へ`schema_migrations`テーブルを追加し、`PRAGMA user_version = 2`へ変更する。初期化トランザクション内でRust側からversion 1の`phase1_baseline`行とversion 2の`schema_migrations`行を現在UTC時刻付きで追加するため、SQLファイルには時刻を固定した履歴データを書かない。

- [ ] **Step 5: Migration実行器の単体テストをgreenにする**

Run: `mise exec -- cargo test store::migrations`

Expected: rollbackテストがPASSし、失敗したMigrationが作成したテーブルとversion変更がTransaction drop後に残らない。

### Task 3: Database openへMigrationと未知versionエラーを統合

**Files:**
- Modify: `src/error.rs`
- Modify: `src/output.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/store/schema.sql`
- Test: `tests/cli_init.rs`、`src/error.rs`

**Interfaces:**
- Consumes: Task 2のversion検証、固定Migration配列、`apply_pending`。
- Produces: `AppError::UnsupportedDatabaseSchemaVersion { found_version: u32, current_version: u32 }`、schema version 1/2を受け入れる`Database::open`、Migration成功時の`schema_migrate`監査イベント。

- [ ] **Step 1: 未知versionエラーの失敗テストをgreenにする**

`AppError`へ`UnsupportedDatabaseSchemaVersion`を追加し、終了コード2、コード`unsupported_database_schema_version`、表示文`database schema version {found_version} is unsupported; current version is {current_version}`を実装する。`output::write_error`では次のdetailsを追加する。

```json
{
  "found_version": 99,
  "current_version": 2
}
```

`src/error.rs`の既存エラーテストへ、終了コード・code・Displayの組み合わせを検証するテストを追加してから実装する。

- [ ] **Step 2: header preflightのversion判定を実装する**

`Database::open`は、まずheaderからapplication IDを検査し、bettr application IDの場合だけ`migrations::is_supported_version`を呼ぶ。application ID不一致は`database_not_initialized`、version 0・3以上・その他未知versionは`unsupported_database_schema_version`とする。未知versionではread-write接続、WAL設定、監査書き込みを実行しない。

- [ ] **Step 3: 同一接続の再検査とMigration実行を実装する**

`open_verified`で接続後のidentityを再検査し、既知versionであることを確認してから既存の接続設定を適用する。versionが1の場合は`BEGIN IMMEDIATE`で`apply_pending`を呼び、実行器がロック取得後の`user_version`を再読込する。返されたMigrationごとに`schema_migrate`監査を同一Transactionへ追加し、最後にcommitする。監査contextはsystem、target/project/revisionなし、changed fields空配列、metadataは次の形に固定する。

```json
{
  "from_version": 1,
  "to_version": 2,
  "migration": "schema_migrations"
}
```

Migration関数、履歴追加、`user_version`更新、監査挿入、commitのいずれかが失敗した場合はTransactionをdropして全変更をrollbackする。

- [ ] **Step 4: 初期化時に履歴を追加する**

`initialize_schema`で`schema.sql`適用直後、`init`成功監査を追加する前に、version 1・2の`schema_migrations`履歴行を同じ初期化Transactionへ追加する。既存の`init`監査は1件のままにし、初期化時に`schema_migrate`を別途記録しない。

- [ ] **Step 5: 初期化・version 1移行・未知versionテストを実行する**

Run: `mise exec -- cargo test --test cli_init --test phase1_workflow`

Expected: 新規DBがversion 2と履歴2行になり、version 1 fixtureが既存データを保持してversion 2へ移行し、未知versionはexit 2とdetailsを返して元ファイルを変更しない。

### Task 4: 複数プロセスのMigration回帰テストとJSON文書を追加

**Files:**
- Modify: `tests/sqlite_concurrency.rs`
- Modify: `docs/json-contract.md`
- Modify: `README.md`（database safety boundaryのschema version記述のみ）

**Interfaces:**
- Consumes: Task 3の`Database::open` Migration処理と`schema_migrate`監査。
- Produces: 同時起動で履歴が一度だけ追加される回帰テストと、未知versionエラーの利用者向け契約。

- [ ] **Step 1: version 1 fixtureを作るテストヘルパーを書く**

`tests/sqlite_concurrency.rs`へ次のヘルパーを追加し、初期化済みDBから`schema_migrations`を削除して`user_version`を1へ戻す。`DROP TABLE IF EXISTS`を使い、実装前のversion 1 schemaでもhelper自体が失敗しないようにする。

```rust
fn downgrade_to_schema_version_one(app: &crate::support::TestApp) {
    let connection = rusqlite::Connection::open(&app.database).unwrap();
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS schema_migrations;\n\
             PRAGMA user_version = 1;",
        )
        .unwrap();
}
```

- [ ] **Step 2: stale versionと同時MigrationのテストをREDで追加する**

Migration実行器のunit testでは、1つ目のTransactionをcommitした後、古いversion 1を渡して2つ目のTransactionを開始してもno-opになることを検証する。さらに`GatedBettr`を2つ起動し、両方に`project list --json`を渡して同じversion 1 fixtureを同時に開かせる。両プロセス終了後にversion 2、履歴version 1・2の各1行、`schema_migrate`監査1件、`PRAGMA integrity_check = 'ok'`、`PRAGMA foreign_key_check`空を検証する。

- [ ] **Step 3: 並行テストを実行して失敗を確認する**

Run: `mise exec -- cargo test --test sqlite_concurrency concurrent_database_opens_apply_schema_migration_once -- --nocapture`

Expected: 実装前は`schema_migrations`が存在しないためFAILする。

- [ ] **Step 4: JSON契約を更新する**

`docs/json-contract.md`のexit code表へ`unsupported_database_schema_version`を追加し、既知の移行元versionは自動更新され、未知versionはexit 2で拒否されること、JSON responseの`schema_version: 1`とは異なることを記述する。READMEの安全境界も「現在versionと互換性のある既知versionを確認する」表現へ更新する。

- [ ] **Step 5: 対象テストをgreenにする**

Run: `mise exec -- cargo test --test sqlite_concurrency --test cli_init --test phase1_workflow`

Expected: 移行、rollback、未知version、監査、並行アクセスの対象テストがすべてPASSする。

### Task 5: 全体検証とIssue記録

**Files:**
- Modify: `docs/superpowers/plans/2026-08-16-schema-migration-foundation.md`（実施チェックボックスのみ）
- Modify: bettr Issue #2（コメント・状態）

**Interfaces:**
- Consumes: Task 1〜4の実装とテスト。
- Produces: clippy・全テスト・diff検証の証拠、Issue #2の完了記録。

- [ ] **Step 1: formatとclippyを実行する**

Run: `mise exec -- cargo fmt -- --check`

Run: `mise exec -- cargo clippy --all-targets --all-features -- -D warnings`

Expected: format差分なし、clippy警告なし。

- [ ] **Step 2: 全テストを実行する**

Run: `mise exec -- cargo test`

Expected: 全unit test・integration testがexit code 0で終了し、失敗0件。

- [ ] **Step 3: 変更範囲と要件対応を確認する**

Run: `git diff --check`

Run: `git status --short`

Issue #2の各完了条件について、対応するテストを次のように照合する。

| 完了条件 | 検証箇所 |
| --- | --- |
| version 1の履歴と適用状態 | `cli_init`の初期化・移行テスト |
| 単一トランザクションとrollback | `store::migrations`の失敗テスト、Migration統合 |
| 未知versionの安全な拒否 | `cli_init`のbytes・sidecar・JSONテスト |
| 同時起動で二重適用しない | `sqlite_concurrency`の同時Migrationテスト |
| JSON契約・監査イベント | `error`テスト、`cli_init`の`schema_migrate`監査検証 |

- [ ] **Step 4: 実装結果をbettrへコメントする**

コメントには変更概要、設計文書・実装計画の保存先、検証コマンドの結果、専用ブランチ名を記録する。生のコマンドラインやIssue本文をコメントへ複製しない。

- [ ] **Step 5: 検証後にIssueをcompleteへ更新する**

`bettr issue show 2`で最新revisionを取得し、検証結果を`summary`と`verification`へ記録して`complete`へ遷移する。revision conflictが出た場合は現在Issueを再読込し、既存コメントを上書きせず新しいコメントとして状況を記録する。
