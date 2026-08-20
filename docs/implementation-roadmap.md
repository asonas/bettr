# bettr Implementation Roadmap

## 方針

bettrは、各段階が単独で利用・検証できる三つのPhaseに分けて実装します。Phase 1でIssueトラッカーとしての最小の縦切りを完成させ、Phase 2で複数エージェントの協調を追加し、Phase 3で外部監査と運用保守を完成させます。Web UI、ネットワーク共有、SQLite以外のデータベースは、この三つのPhaseを実運用した後に判断します。

## Phase 1: Local Issue Core

Phase 1では、プロジェクト、Issue、コメント、5状態、履歴、実行コンテキスト、SQLite内の追記専用監査イベント、JSON出力を実装します。CLIは非対話式とし、複数プロセスによる読み書き、busy timeout、楽観的ロック、安定した終了コードを含めます。人間は`bettr status`で全プロジェクトの停止状況と進行状況を確認できます。

完了条件は、空の環境で`bettr init`からプロジェクト作成、Issue作成、着手、block、再開、完了または中止、コメント追加、履歴確認までを実行でき、同時更新で静かな上書きが発生せず、全操作がSQLite内の監査イベントへ残ることです。詳細は`docs/superpowers/plans/2026-08-15-phase-1-local-issue-core.md`に記載します。

## Phase 2: Agent Coordination

Phase 2では、原子的なclaim、セッション単位のlease、heartbeat、stale表示、理由付きtakeover、複数の判断要求と人間の判断イベント、Issue間の`blocks`依存、一階層の親子関係、構造化reference、イベントカーソル、`bettr capabilities --json`を実装します。冪等性キーと原子的なJSONバッチ更新は、別のcapabilityとして未実装のまま明示します。

完了条件は、二つのプロセスが同じIssueを同時にclaimできず、期限切れleaseが自動的に再割当されず、判断要求と回答が個別に対応し、wayfinderがカーソル以降の差分だけを取り込めることです。実装済みのCLIとskillの対応は[`contracts/capabilities.json`](../contracts/capabilities.json)で確認できます。

## Phase 3: Audit and Operations

Phase 3では、SQLiteを正本とする追記専用JSON Lines監査ログ、ハッシュチェーン、未反映イベントの復旧を段階的に実装します。Issue #9は通常のCLI実行に伴う安全な自動投影、SQLite cursorによる追記直列化、クラッシュ復旧、ローテーション境界を対象とし、Issue #10はarchive、verify、SQLiteからの再構築を対象とします。監査履歴を考慮した機密情報消去と保持期間はIssue #12、SQLite online backupと`bettr doctor`は別の後続Issueで扱います。

Issue #9の完了条件は、プロセス停止を挟んでも監査JSONLをSQLiteから復旧でき、複数プロセスで重複なく追記でき、生の入力や秘密値を出力しないことです。改変・欠落の検知、バックアップ、履歴消去、保持期間は各後続Issueの完了条件とします。

## Local Web UI

Web UIはネットワーク共有を目的とせず、`bettr web`でloopbackにだけバインドする監督ビューとして提供します。Projectsの5列Kanban、プロジェクト別サイドバー、Issue詳細のActivity、待機理由・待機種別・判断要求、既存human decisionの解決、ポーリングによる更新インジケーターを提供します。Webの書き込みは表示したrevisionを使うdecision解決だけに限定し、Issueの編集・claim・コメントなどはCLIから行います。初期実装はRust標準ライブラリのHTTPサーバーと埋め込みVanilla JavaScriptを使います。更新はヘッダーの件数メニューとカード左端のシアン色で示し、通知バナーは使いません。DOM描画・ポーリング・フォーカスはVitest + jsdomで、状態投影の不変条件はNode testでテストし、RustテストはHTTP/API境界に限定します。必要な検索・フィルター・イベントカーソルは別計画に分けます。

## Skills and Distribution

各Phaseと同時にCodexおよびClaude Code向けの最小スキルを更新します。スキルは利用可能なcapabilityだけを使い、Issue化の基準、競合時の再読込、完了根拠の記録を定めます。配布はMIT Licenseの下でGitHub Releasesから行います。Phase 1の間は`cargo install --path .`によるローカル利用も維持します。GitHub Releasesの初期配布は、macOS/Linuxのx86_64・aarch64向けarchive、SHA-256、artifact attestationをタグ駆動workflowで提供します。HomebrewやOS-native installerは後続Issueの範囲です。

## Performance Gate

一般的なローカルSSD上で、起動時間を含む単純な読み書きのp95を50ミリ秒未満とします。10万Issueに対する絞り込み一覧のp95は200ミリ秒未満を目標とします。保証値として公開する前に、固定データセットとリリースビルドによるベンチマークで確認します。
