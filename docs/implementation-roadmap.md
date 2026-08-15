# bettr Implementation Roadmap

## 方針

bettrは、各段階が単独で利用・検証できる三つのPhaseに分けて実装します。Phase 1でIssueトラッカーとしての最小の縦切りを完成させ、Phase 2で複数エージェントの協調を追加し、Phase 3で外部監査と運用保守を完成させます。Web UI、ネットワーク共有、SQLite以外のデータベースは、この三つのPhaseを実運用した後に判断します。

## Phase 1: Local Issue Core

Phase 1では、プロジェクト、Issue、コメント、5状態、履歴、実行コンテキスト、SQLite内の追記専用監査イベント、JSON出力を実装します。CLIは非対話式とし、複数プロセスによる読み書き、busy timeout、楽観的ロック、安定した終了コードを含めます。人間は`bettr status`で全プロジェクトの停止状況と進行状況を確認できます。

完了条件は、空の環境で`bettr init`からプロジェクト作成、Issue作成、着手、block、再開、完了または中止、コメント追加、履歴確認までを実行でき、同時更新で静かな上書きが発生せず、全操作がSQLite内の監査イベントへ残ることです。詳細は`docs/superpowers/plans/2026-08-15-phase-1-local-issue-core.md`に記載します。

## Phase 2: Agent Coordination

Phase 2では、原子的なclaim、セッション単位のlease、heartbeat、stale表示、理由付きtakeoverを追加します。また、複数の判断要求と人間の判断イベント、Issue間の`blocks`依存、一階層の親子関係、構造化reference、イベントカーソル、冪等性キー、原子的なJSONバッチ更新、`bettr capabilities --json`を実装します。

完了条件は、二つのプロセスが同じIssueを同時にclaimできず、期限切れleaseが自動的に再割当されず、判断要求と回答が個別に対応し、wayfinderがカーソル以降の差分だけを取り込めることです。Phase 1を実際の作業に使い、CLIの名称とJSON契約に必要な修正を反映した後、独立した詳細計画を作成します。

## Phase 3: Audit and Operations

Phase 3では、SQLiteを正本とする追記専用JSON Lines監査ログ、ハッシュチェーン、未反映イベントの復旧、archive、verify、再構築を実装します。SQLite online backupによるバックアップと復元、監査履歴を考慮した機密情報消去、`bettr doctor`による整合性・権限・lease・監査検査も追加します。

完了条件は、プロセス停止を挟んでも監査JSONLをSQLiteから復旧でき、改変と欠落を検知でき、稼働中のDBを安全にバックアップでき、秘密内容を消去しても消去操作そのものは追跡できることです。Phase 2までのデータ量と障害事例を確認してから、独立した詳細計画を作成します。

## Skills and Distribution

各Phaseと同時にCodexおよびClaude Code向けの最小スキルを更新します。スキルは利用可能なcapabilityだけを使い、Issue化の基準、競合時の再読込、完了根拠の記録を定めます。配布はMIT Licenseの下でGitHub Releasesから行います。Phase 1の間は`cargo install --path .`によるローカル利用も維持します。

## Performance Gate

一般的なローカルSSD上で、起動時間を含む単純な読み書きのp95を50ミリ秒未満とします。10万Issueに対する絞り込み一覧のp95は200ミリ秒未満を目標とします。保証値として公開する前に、固定データセットとリリースビルドによるベンチマークで確認します。
