# 追記専用JSONL監査ログ 設計

## 目的

SQLiteに保存される監査イベントを正本として、外部連携用の追記専用JSON Linesログへ自動投影する。
CLIの成功・失敗、読み取り・書き込みを一つの監査イベントにつき一行で出力し、プロセス停止やファイル書き込み失敗の後でも、次回のCLI実行で未反映イベントを復旧できるようにする。

この設計は、現在のmain基線 `a84fed6` にあるSQLite監査、schema migration、event cursor、SQLiteのwriter lockを前提にする。
JSONレスポンスの`schema_version: 1`とSQLiteのschema versionは別の契約として維持する。

## 対象範囲

### 実装するもの

- `audit_events`に投影順序を示す単調増加sequenceを追加する。
- SQLiteにJSONL投影cursorと直前hashを保存する。
- 既存の監査イベントをv5 migrationで投影可能にする。
- 各CLI実行の終了後に未反映監査イベントを自動投影する。
- JSONLの安全なイベント形、SHA-256 hash chain、部分行復旧、並行プロセスの直列化を実装する。
- ログファイルが外部ローテーションで置き換えられた場合も、SQLiteに保存した直前hashからチェーンを継続する。
- `audit_jsonl` capabilityとJSON契約・利用方法を追加する。

### 実装しないもの

- `audit verify`、`audit archive`、SQLiteからの`rebuild`。
- 自動ローテーションの世代管理。
- 監査・Issue・コメント・バックアップのredaction。
- SQLite online backup、復元、`doctor`。
- JSONLを明示的に出力する新しいexportコマンド。
- 生のargv、Issue本文、コメント本文、idempotency key、秘密値の保存。

上記のverify/archive/rebuildは#10、redactionと保持方針は#12の範囲であり、このIssueでは先取りしない。

## 設計上の選択

### SQLite cursor方式

JSONL用のpayloadを別outboxテーブルへ複製せず、既存の`audit_events`を読み取り元にする。
`audit_events.sequence`と`audit_jsonl_cursor`が未反映範囲を表すため、SQLite内に同じイベント内容を二重保存しない。

event cursorと同じく、cursorは排他的な整数として扱う。
`sequence > cursor.sequence`の行をsequence昇順で取得し、ファイルへの追記とcursor更新を一つのSQLite writer transactionで直列化する。

### ログパス

明示的な設定やCLIオプションは追加せず、データベースパスの拡張子を`.audit.jsonl`へ置き換えたパスを既定値にする。
例えば`/var/data/bettr.db`の監査ログは`/var/data/bettr.audit.jsonl`となる。
親ディレクトリは既存のデータベース初期化と同じowner-onlyの扱いにし、新規ファイルはUnixではowner read/writeで作成する。

### 自動flushの位置

`run`はCLI処理本体を内側のResultへ閉じ込め、処理が成功または失敗した後に、解決済みデータベースを再度開いてJSONL flushを実行する。
これにより、通常の成功監査、アプリケーションエラーを記録した失敗監査、`init`で初めて作られた監査を同じ終了処理で投影できる。

SQLiteを開けない、またはデータベースが未初期化でSQLite監査自体を作れないCLIエラーには、投影可能な正本イベントがないためJSONL行を作らない。
解析済みCLIエラーを既存の`audit_unparsed_cli_failure`がSQLiteへ記録できた場合は、その後のflush対象にする。

外部JSONLのflush失敗はSQLiteの正本イベントやCLIのドメイン結果を変更しない。
cursorは進めず、次回実行で再試行し、標準エラーには安全なエラーコードだけを出力する。

## SQLiteスキーマ

### `audit_events.sequence`

新規初期化のschema version 5では`audit_events`に次の列を含める。

```sql
sequence INTEGER NOT NULL UNIQUE
```

新しい監査行は、既存の`BEGIN IMMEDIATE`トランザクション内で`MAX(sequence) + 1`を割り当てる。
すべての監査挿入がwriter transaction内で行われるため、複数プロセスが同じsequenceを取得することはない。

既存v4 DBのmigrationでは、SQLiteが既存行のあるテーブルへ`NOT NULL`列を直接追加できないため、次の順序で適用する。

1. `sequence INTEGER`をnullableな列として追加する。
2. 既存行をSQLiteのrowid順に並べ、既存の順序を保つ値としてsequenceをbackfillする。
3. sequenceのunique indexを作成し、以後のbettrの監査挿入は常に値を指定する。

backfillとindex作成はmigration transaction内で行うため、コミット後のbettr管理下の行にNULLは存在しない。
新規初期化では最初から`NOT NULL UNIQUE`を適用し、将来の監査挿入でもNULLを許可しない。
既存の監査内容、UUID、時刻、結果、実行コンテキストは変更しない。

### `audit_jsonl_cursor`

```sql
CREATE TABLE audit_jsonl_cursor (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    previous_hash TEXT,
    updated_at TEXT NOT NULL
);
```

常にidが1の一行を持つ。
`sequence`は最後にJSONLへ反映したsequence、`previous_hash`はその行のhashであり、まだ一行も出力していない場合は`sequence = 0`かつ`previous_hash = NULL`とする。

v5 migrationは既存監査行を削除せず、cursorを0から開始する。
新規初期化は最新スキーマとcursor初期行を同じ初期化transactionで作成する。

MigrationのDDL、既存sequenceのbackfill、cursor初期化、`user_version`更新、migration履歴は既存migration実行器のtransaction契約に従う。

## JSONLイベント契約

一行は次のフィールドを持つJSON objectとする。

```json
{
  "schema_version": 1,
  "sequence": 42,
  "event_id": "uuid",
  "started_at": "2026-08-20T07:00:00Z",
  "finished_at": "2026-08-20T07:00:01Z",
  "operation": "issue_create",
  "context": {
    "kind": "agent",
    "agent": "codex",
    "session_id": "session-1"
  },
  "project_id": "uuid",
  "target": {
    "kind": "issue",
    "id": "uuid"
  },
  "revision": 1,
  "changed_fields": ["title"],
  "result": {
    "outcome": "success",
    "exit_code": 0
  },
  "previous_hash": null,
  "hash": "sha256-hex"
}
```

`project_id`、`target`、`revision`は監査行に値がある場合だけ含める。
`context`は既存の監査列から再構成し、agentでは`agent`と`session_id`、humanでは`operator`、systemでは`kind`だけを保存する。
`result.outcome`は`success`または`failure`、`result.exit_code`は既存の監査終了コードと一致させる。
失敗監査に保存された制御済みの`error_code`だけは`result.error_code`として含める。

`changed_fields`は既存のoperation別allowlistを通った値だけを使う。
`project_name`、`idempotency_key`、監査metadata全体、処理結果のresponse、raw argvはJSONLへコピーしない。
これにより、Issue title/body、コメント本文、transition metadata、batch入力、秘密値がJSONLへ流れ込まない。

## Hash chain

`hash`を除くイベントフィールドを、Rustの固定フィールド順のserde serializationで一行JSON化し、`previous_hash`を含むUTF-8 bytesへSHA-256を適用する。
hashは小文字hexで保存する。
一行目の`previous_hash`はJSON null、二行目以降はcursorの`previous_hash`を使う。
改行文字はhash計算へ含めず、ファイルへ書くときだけ末尾に一つ付ける。

このIssueでは全履歴のverifyは行わないが、各行にsequence・previous_hash・hashを残すことで#10が改変、欠落、重複を検査できる。

## Flushと障害復旧

flushは次の順序で行う。

1. `BEGIN IMMEDIATE`でSQLite writer lockを取得する。
2. cursorより後の`audit_events`をsequence昇順で読み、安全なJSONLイベントを構成する。
3. ログファイルの末尾に改行がない場合、最後の改行までを残して部分行を切り詰める。
4. 次に出力するイベントと同じsequence・event_id・hashの完全な末尾行が既にあれば、二重追記せずcursor更新だけを行う。
5. それ以外は一行をappendし、`sync_data`でファイル内容を同期する。
6. cursorのsequence、previous_hash、updated_atを更新し、SQLite transactionをcommitする。

ファイルへのappend後、SQLite commit前にプロセスが停止した場合、cursor transactionはrollbackする。
次回flushは既存の完全な末尾行を照合して追記を省略するため、同じ監査イベントを二重出力しない。
途中までの行だけが残った場合は、その行を切り捨ててから同じイベントを書き直す。

末尾行が次に期待するsequenceと一致しない場合は、既存ファイルを上書きせずflushを失敗させる。
cursorは進めないため、ファイルを確認・交換した後の次回実行で復旧できる。

SQLite writer lockを保持したままファイル操作を行うことで、bettrプロセス同士のappend順序とcursor更新を直列化する。
bettr外部のプロセスが同じファイルへ書き込むことはこのIssueでは同期対象にしない。

## ローテーション境界

自動ローテーションとarchive操作は実装しない。
既存のJSONLファイルがrenameまたは削除され、同じパスに新しいファイルが作られた場合は、SQLite cursorに残るprevious_hashを新しいファイルの最初の行へ設定する。
これによりファイル単位の境界をまたいでもチェーンの接続情報を失わない。
旧ファイルの世代管理、archive metadata、チェーン全体の検査は#10で扱う。

## Capabilityと既存API

`bettr capabilities --json`と共有capability契約へ`audit_jsonl: true`を追加する。
既存の`audit list`、`event list`、`issue history`のJSON形は変更しない。
既存の`domain_events`とそのevent cursorはIssue履歴・wayfinder用のまま維持し、JSONL監査のsequenceをdomain eventへ混ぜない。
新しいexport、verify、archive、rebuildのCLIサブコマンドは追加しない。

## テスト計画

次のテストを既存のRust integration/unit test構成へ追加する。

1. 新規初期化がschema version 5、migration履歴、cursor初期行、監査sequenceを作ること。
2. v4データベースのmigrationが既存監査sequenceをrowid順にbackfillし、既存データを変更しないこと。
3. 成功・失敗、読み取り・書き込みの監査行が、JSONLでは一監査行一JSON行になること。
4. JSONLのschema version、sequence、context、result、previous_hash、hashが契約どおりであること。
5. Issue title/body、コメント本文、raw argv、idempotency keyに含めた秘密値がJSONLへ現れないこと。
6. 複数行のhash chainとSHA-256計算が検証できること。
7. 末尾の部分JSON行と、append後commit前を表す完全な重複末尾行から、次回flushが重複なく復旧すること。
8. 旧ログのrename後に新しいログへ追記してもprevious_hashが継続すること。
9. 複数プロセスが同時に監査を生成しても、JSONLに重複、interleave、途中行がなく、sequenceが昇順になること。
10. flush失敗時にcursorが進まず、CLIのドメイン結果を変更せず、次回実行で再試行できること。

全テストはRust runtimeの規約に従い`mise exec -- cargo ...`で実行する。

## 実装対象

- `src/store/migrations.rs`: v5 migration、sequence backfill、cursor schema
- `src/store/schema.sql`: 最新スキーマと初期cursor
- `src/store/sqlite.rs`: sequence割当、安全なprojection、hash、flush、復旧
- `src/main.rs`: CLI処理後のflush境界と安全なflushエラー処理
- `src/app.rs`、`src/output.rs`: 必要な安全な結果型・診断の接続
- `src/domain.rs`または監査専用型: JSONL schemaの固定serialization
- `contracts/capabilities.json`、`docs/json-contract.md`、`README.md`: capabilityと利用契約
- `skills/bettr/SKILL.md`、`skills/bettr-claude/SKILL.md`: `audit_jsonl` capabilityの反映
- 既存のmigration/audit/concurrency testおよびJSONL専用integration test

実装はこの設計に必要な範囲だけとし、#10/#12の機能や将来の設定項目を追加しない。
