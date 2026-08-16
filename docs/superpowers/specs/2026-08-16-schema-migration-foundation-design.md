# SQLiteスキーマ移行基盤 設計

## 目的

Issue #2では、Phase 1で作成されたschema version 1のSQLiteデータベースを安全に更新し、Phase 2以降のテーブル・列追加を一貫した仕組みで適用できるようにする。

対象は既存のローカルSQLiteデータベースであり、ネットワーク越しのデータベース、外部のMigrationサービス、暗黙のデータ修復は対象外とする。

## 現状と問題

現在の`Database::open`は、SQLite headerの`application_id`と`user_version`がそれぞれbettr固有値と1である場合だけデータベースを開く。`schema.sql`もschema version 1を直接作成しており、既存データベースを次のスキーマへ更新する経路がない。

また、Migrationの適用履歴を保持するテーブルがなく、複数プロセスが同時に起動した場合に、適用済み変更を再適用せずに確認する仕組みもない。

## 採用する設計

### スキーマバージョン

SQLiteのデータベースschema versionは2へ進める。JSONレスポンスの`schema_version: 1`とは別の値であり、JSON契約のversionは変更しない。

bettrの`application_id`は変更しない。既存のschema version 1は互換性のある移行元として受け入れ、schema version 2を現在の最新バージョンとする。application IDが異なるファイルは、これまでどおり`database_not_initialized`として拒否する。

### Migration履歴

最新スキーマに次のテーブルを追加する。

```sql
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
);
```

適用済みMigrationを1行ずつ保存し、SQLiteの`PRAGMA user_version`と履歴行の最大versionを現在の適用状態として扱う。Migrationの変更、履歴行の追加、`user_version`の更新は同じトランザクションで行うため、失敗したMigrationの履歴だけが残る状態は作らない。

schema version 1から2へのMigrationは、`schema_migrations`テーブルを作成し、既存のschema version 1を`phase1_baseline`として記録したうえで、Migration version 2を記録する。新規初期化では、最新スキーマとversion 1・2の履歴を初期化トランザクション内で作成する。

Migration名はコード上の固定値とし、Issue #2ではchecksumや手動編集用の状態値は導入しない。Migrationの適用済み状態は、履歴行の存在と`user_version`によって十分に判断できる。

### 起動時の処理順序

既存データベースを開く処理は次の順序にする。

1. SQLite headerを副作用なく読み、SQLite形式とapplication IDを検査する。
2. schema versionが既知の移行元（1）または最新（2）であることを確認する。未知のversionはSQLite接続を開かずに拒否する。
3. read-write接続を開き、同じ接続上でapplication IDとschema versionを再確認する。
4. 既存のbusy timeout、foreign keys、WAL設定を適用する。
5. schema versionが最新でなければ、`BEGIN IMMEDIATE`相当のトランザクションで未適用Migrationを順番に実行する。
6. 各Migrationの変更、履歴記録、`user_version`更新、Migration監査イベントを同じトランザクションへ含めてcommitする。
7. すでに最新であればデータを変更せず、そのまま利用する。

`BEGIN IMMEDIATE`により、複数プロセスが同時にschema version 1を開いても、最初のプロセスだけがMigrationを適用する。後続プロセスはbusy timeoutの後にトランザクションを開始し、その時点の`user_version`と履歴を再読込して適用済みと判断する。ロック取得に失敗した場合は既存の`database_busy`契約に従う。

新規初期化は、従来どおり作成直後のファイルを単一の初期化トランザクションで構築する。ただし作成するスキーマと履歴は最新version 2とする。

### Migration実行器

Migrationはversion、固定名、トランザクションへDDL/DMLを適用する関数からなるコード上の登録情報として管理する。実行器は次の責務だけを持つ。

- 現在versionより大きく、最新version以下のMigrationを昇順に選択する。
- Migrationを実行する。
- 成功したMigrationの履歴を追加する。
- `PRAGMA user_version`を更新する。
- いずれかの処理が失敗したらトランザクション全体をrollbackする。

Migration登録情報には、将来の機能を先回りしたCLIや外部設定を追加しない。新しいテーブル・列が必要になったときに、同じ実行器へMigrationを追加できる最小構成にする。

### 未知のschema version

application IDがbettrのもので、schema versionが既知の範囲外の場合は、`unsupported_database_schema_version`エラーを返す。エラーには発見したversionと現在の最新versionをJSONの`details`として含め、終了コードは入力されたデータベースを利用できないことを示す2とする。

未知のversionではSQLiteのread-write接続、WAL設定、Migration、監査書き込みを行わない。これにより、元ファイルのbytesやSQLite sidecarを変更せず、未来のデータベースを破壊的に開かない。

### 監査

Migrationが実行された場合、`schema_migrate`操作としてSQLiteの`audit_events`へ記録する。監査metadataにはMigration名、移行元version、移行先versionだけを保存し、Issue本文・コメント本文・生のコマンドライン・秘密値は保存しない。Migration変更、履歴行、`user_version`、監査イベントは一つのトランザクションに含める。

Migration前に失敗した未知versionの拒否は、データベースを安全に開けないため、既存のIssue監査へ記録しない。エラーはCLIのJSON契約で直接返す。

## エラーとJSON契約

既存のレスポンス契約`schema_version: 1`は維持する。`unsupported_database_schema_version`をエラーコードとして追加し、JSON例を`docs/json-contract.md`へ追加する。

既存の`database_not_initialized`は、非SQLiteファイル、別用途のSQLite、bettr application IDを持たないSQLiteに引き続き使用する。

## テスト計画

完了条件を次のテストで検証する。

1. 新規初期化でschema version 2となり、version 1・2の履歴が保存される。
2. schema version 1のfixtureを開くと、既存のprojects、issues、comments、domain_events、audit_eventsを保持したままversion 2へ移行する。
3. Migration関数が失敗した場合、変更されたテーブル、履歴行、`user_version`、監査イベントがすべてrollbackされる。
4. 未来のschema versionを持つbettr databaseは、`unsupported_database_schema_version`とdetailsを返し、元bytesとsidecarを変更しない。
5. 複数プロセスがschema version 1を同時に開いても、Migration履歴とschema変更が一度だけ適用され、SQLite integrity checkとforeign key checkが成功する。
6. Migration成功時の監査イベントが、metadataの許可された値だけを含む。

テストは既存の`cli_init`、`sqlite_concurrency`の構成を活用し、Migration実行器のrollback検証には実行器へ失敗関数を渡すユニットテストを追加する。将来のMigrationを先回りしたテスト用APIやCLIは追加しない。

## 変更対象

- `src/store/migrations.rs`: Migration定義と実行器
- `src/store/mod.rs`: Migrationモジュールの登録
- `src/store/sqlite.rs`: schema version判定、接続後のMigration実行、初期化との統合
- `src/store/schema.sql`: 最新スキーマとMigration履歴の初期値
- `src/error.rs`、`src/output.rs`: 未知schema versionのエラー契約
- `docs/json-contract.md`: エラーコードとdatabase schema versionの説明
- `tests/cli_init.rs`、`tests/sqlite_concurrency.rs`、必要なMigrationテスト: 完了条件の回帰検証

Issue #2でclaim、lease、decision request、依存関係、batch更新、JSONL監査などのPhase 2・3機能は実装しない。
