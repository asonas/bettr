# wayfinder Phase 2 workflow

この例は、wayfinderが`bettr`を直接呼び出してローカルのエージェント作業を調整する契約を示す。
wayfinder本体はこのリポジトリに含めない。

## 1. Capability gate

起動時に一度、JSON契約とcapabilityを確認する。

```sh
bettr capabilities --json
```

`schema_version`と`data.json_contract_version`が対応していなければ停止する。
`data.capabilities`で`true`の機能だけを使い、未知の名前や`false`の機能からコマンドを推測しない。
Phase 2で利用できるのは、`issue_dependencies`、`issue_parent`、`issue_claim`、`issue_lease`、`human_decisions`、`event_cursor`、`capabilities`である。

## 2. Agent context and claim

エージェントごとに作業単位のセッションIDを生成し、子プロセスへ渡す。

```sh
export BETTR_AGENT=codex
export BETTR_SESSION_ID=wayfinder-<work-unit>
bettr issue claim --project bettr --json
```

返却された`data.issue.revision`を保存する。leaseの期限までに必要な間隔でheartbeatを送る。
heartbeatはIssue revisionを増やさない。

```sh
bettr issue heartbeat <number> --project bettr --json
```

claimに失敗したら、Issueを再選択する。staleなIssueを自動再割当しない。
期限切れleaseを引き継ぐ場合だけ、前セッションの状態を確認し、理由を付けてtakeoverする。

## 3. Decision stop

作業中に人間の判断が必要になったら、問いと背景を保存してそのIssueの実行を停止する。

```sh
bettr decision request <number> --project bettr \
  --question "どちらの互換動作を採用するか" \
  --background "選択によって移行手順が変わる" --json
```

request UUIDを保存し、同じagent/sessionで解決しようとしない。
人間が次状態を明示して解決した後、`status --json`で`attention`から消えたことを確認して再開する。
未解決requestがある間は完了操作を送らない。

`next-state`に応じて遷移メタデータを渡す。`blocked`は`--reason`と`--wait-kind`、`done`は`--summary`と`--verification`、`cancelled`は`--reason`が必須である。作業を再開する場合は`todo`で解決し、agent claimをやり直す。

## 4. Exclusive event polling

永続化したカーソルを`cursor`とし、最後に処理したsequenceの後だけを取得する。

```sh
bettr event list --after <cursor> --limit 100 --include-issue --json
```

イベントをsequence順に処理し、処理が成功した後でだけ`next_cursor`を保存する。
`has_more`がtrueなら同じ`next_cursor`から続けて読む。
heartbeat、読み取り、失敗はdomain eventではないため、イベント欠落として扱わない。
各読み取りは一つのSQLite snapshotから返る。

## 5. Verified completion

変更前にIssueを読み直し、revisionを使って完了する。

```sh
bettr issue show <number> --project bettr --json
bettr issue complete <number> --project bettr --revision <revision> \
  --summary "実装した" \
  --verification "mise exec -- cargo test" --json
```

`revision_conflict`なら古い書き込みを再送せず、再読込して差分を判断する。
結果不明の書き込みは`issue show`、`issue history`、`audit list`で確認してから再試行する。

## Installation boundary

wayfinderの配布物には、`bettr`のバイナリとこの契約に対応するskillを同じ検証済みリビジョンから含める。
実行時に`capabilities --json`を再確認し、別リビジョンのCLIへ暗黙にフォールバックしない。
