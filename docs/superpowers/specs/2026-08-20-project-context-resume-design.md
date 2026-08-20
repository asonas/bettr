# リポジトリ文脈からのbettrプロジェクト解決

## 目的

作業再開時に、別リポジトリのIssueや全プロジェクトのstatusから対象プロジェクトを推測しない。現在のリポジトリが明示的に紐付いている場合だけ、そのプロジェクトを作業対象として扱い、紐付けがない場所ではユーザーに確認する。

## 設計

### リポジトリの紐付け

リポジトリルートに `.bettr.toml` を置き、`project = "<project-name>"` を記録する。bettr CLIが既に持つディレクトリ設定の親探索を利用するため、リポジトリ配下のネストしたディレクトリからも同じプロジェクトを解決できる。

このリポジトリでは次をコミットする。

```toml
project = "bettr"
```

### エージェントの作業再開規則

CodexとClaude Codeのbettrスキルに、次の順序を追加する。

1. `command -v bettr` と `bettr --help` を確認する。
2. `bettr context --json` を実行し、解決された `project.value` と `project.source` を読む。
3. `project.value` が存在する場合だけ、`--project <value>` を付けてIssueを読む。
4. `project.value` がnullの場合、`bettr status` やIssueのpriority、更新時刻、assignee、リポジトリ名からプロジェクトを推測せず、ユーザーに確認する。
5. プロジェクトが解決した後も、複数のIssueが再開候補になる場合はIssue番号を断定せず、候補を提示して確認する。

`bettr status --json` は複数プロジェクトを監督するための読み取り操作として残す。作業再開先を決める入力には使わない。

### CLIの扱い

CLIの既存のプロジェクト解決優先順位（引数、環境変数、最寄りの `.bettr.toml`、ユーザー設定、未解決）を変更しない。今回の変更は、この既存の解決結果を作業再開の安全境界として利用する文書・リポジトリ設定の追加に限定する。

## エラーと曖昧さ

- `.bettr.toml` がないディレクトリでは、project未解決を正常な状態として扱う。
- project未解決時に自動で `status` の全プロジェクトを参照して候補を選ばない。
- projectが解決していても、Issue候補が一意でない場合は確認を要求する。
- `context` が設定エラーを返した場合は、設定を修正するまでIssue操作へ進まない。

## 検証

- `.bettr.toml` を置いたリポジトリルートで `bettr context --json` が `project.value = "bettr"`、`project.source = "directory_config"` を返す。
- リポジトリ配下のネストしたディレクトリでも同じ結果になることを既存のCLIテストで確認する。
- CodexとClaude Codeのスキルが、project未解決時に推測せず確認する規則と、`status` の用途分離を含むことを契約テストで確認する。
- READMEにリポジトリ設定と、設定のない作業ディレクトリでの確認方針を記載する。
- 既存のRust通常テストとスキル契約テストを実行する。`cli_latency` ベンチマークの既存失敗は今回の受け入れ条件に含めない。
