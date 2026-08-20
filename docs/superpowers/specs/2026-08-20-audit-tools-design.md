# 監査JSONLのverify・archive・rebuild設計

## 目的

Issue #9で実装したDB隣接の追記専用JSONL監査ログについて、改変・欠落・重複・途中書き込みを検出し、SQLiteを正本として安全に再構築し、世代交代できるようにする。

SQLiteの`audit_events`と`audit_jsonl_cursor`を正本として再利用する。JSONLは復旧可能な投影物であり、JSONLの内容をSQLiteへ戻す処理は追加しない。

## 人間判断で確定した方針

- active JSONLは、DBパスから導出した`bettr.audit.jsonl`とする。
- archiveはactiveを同じディレクトリの日時付き世代ファイルへ原子的にrenameする。
- archive後の新activeは空で作成し、SQLite cursorの`previous_hash`を保持する。
- 新activeへ最初に投影されるイベントは、退避した世代の末尾hashを`previous_hash`としてチェーンを継続する。
- rebuildは一時ファイルへ生成・検証してからactiveを原子的に置換する。生成または検証に失敗した場合、既存activeは変更しない。

## CLI契約

### `bettr audit verify`

引数なしではactive JSONLを検査する。archive世代を検査する場合は`--path PATH`で対象ファイルを指定する。

検査項目は次のとおり。

- 各行が完全なUTF-8 JSON objectで、末尾に改行があること。
- `schema_version`が1であること。
- sequenceが正の整数で、ファイル内で一つずつ増えること。
- event_idが重複しないこと。
- 各行の`hash`が、`hash`を除くcanonical JSONと一致すること。
- 2行目以降の`previous_hash`が直前行のhashと一致すること。

ファイル先頭のsequenceは1または世代境界後の値を許可する。先頭の`previous_hash`がnullでない場合も許可し、世代ファイルとの接続は各ファイルを跨いだ検査の責務にしない。ファイル内の連続性と各行のhashは常に検査する。

成功時は検査した行数、first sequence、last sequenceをJSONで返す。空ファイルは0行の有効なログとする。

### `bettr audit archive`

active JSONLを`<stem>.<UTC日時>.jsonl`へrenameし、同じactiveパスに空ファイルを作る。UTC日時には秒未満の値を含め、同一プロセス内の連続実行で名前が衝突しないようにする。

archive前にactiveをstrict verifyし、最後のsequenceとhashがSQLite cursorと一致することを確認する。一致しない場合はarchiveせず、rebuildを復旧手順として案内する。activeが空または存在しない場合は世代を作らず、成功結果として`archived: false`を返す。

ファイル操作はSQLiteの`BEGIN IMMEDIATE`中に行う。これによりbettrプロセス同士のarchiveとflushを直列化する。activeの作成に失敗した場合は可能な範囲でrenameを戻し、失敗時に既存activeを失わない。

archiveの操作監査イベントはSQLiteへ保存した後、通常のCLI終了時flushで新activeへ投影する。従って新activeの最初の行はSQLite cursorに残っていた直前hashから始まる。

### `bettr audit rebuild`

SQLiteの`audit_events`をsequence昇順で全件読み、sequence 1から連続していることを確認してactive JSONLを再生成する。範囲指定は追加しない。全件再構築が、欠落を含む部分的な復旧を誤って正本と扱わない最小の契約である。

生成物はUUID付き一時ファイルへ書き、strict verifyしてからactiveへrenameする。SQLite transaction中にcursorを再生成結果の最後のsequence/hashへ更新してcommitする。

rebuild自身の監査イベントは再構築対象のsnapshot取得後にSQLiteへ記録される。そのためrebuild結果には自身のイベントを含めず、CLI終了時の通常flushで最後に追記される。再実行は冪等に同じSQLite範囲を再生成する。

## JSONエラー契約

監査JSONLの内容が壊れている、SQLiteのsequenceが連続していない、activeとcursorが一致しない場合は`audit_integrity_failure`を返す。ファイル操作の失敗は`audit_operation_failed`を返す。いずれも終了コード10とし、機械可読な`error.code`を安定値とする。

`audit_integrity_failure`のdetailsには、判定できる場合だけ行番号とsequenceを含め、復旧方法として「対象JSONLを保存したうえで`bettr audit rebuild --json`を実行する」ことを含める。エラーメッセージ、details、監査JSONLにはargv、Issue本文、コメント本文、秘密値、ファイル内容を含めない。

## 対象外

- archive世代を自動削除するretention。
- SQLite backup・restore・doctor。
- redaction。
- archive世代を全て横断するverifyコマンド。
- JSONLをSQLiteへ取り込む処理。

## テストシーム

公開CLIのJSON入出力とfilesystemをシームとして、次をintegration testで検証する。

1. 正常なactiveとarchive世代のverify。
2. hash改変、sequence欠落・重複、途中書き込みのverify失敗とJSON error code。
3. rebuildがSQLiteから連続した全イベントを再生成し、cursorを合わせること。
4. rebuildの生成・検証失敗時にactiveが保持されること。
5. archiveが世代ファイルを作り、新activeの次回イベントが旧末尾hashから継続すること。
6. archive前にactiveとcursorが不一致の場合、archiveせず復旧手順を返すこと。

