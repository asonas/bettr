# GitHub Releases配布設計

## 目的

`cargo install --path .` に加え、公開リポジトリからmacOSとLinuxのbettrバイナリを、再現可能な手順で取得・検証・更新できるようにする。

## 採用方針

配布workflowはリポジトリ内に明示的なGitHub Actions YAMLとして保持する。`cargo-dist`は導入せず、対象、命名、version検証、権限、Release upload、検証手順をこのリポジトリで管理する。

対象は次の4つに固定する。

| OS | Rust target | GitHub-hosted runner |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `ubuntu-24.04` |
| Linux arm64 | `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| macOS x86_64 | `x86_64-apple-darwin` | `macos-15-intel` |
| macOS arm64 | `aarch64-apple-darwin` | `macos-14` |

各runnerは対象targetと同じCPUアーキテクチャで実行し、cross executionや別のcross compilerに依存しない。将来runner labelが変わる場合はworkflowと検証手順を同時に更新する。

## Workflow

### CI

Pull requestとmainへのpushで、Rust stableを使って次を実行する。

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --locked`
- `cargo build --locked --release`

### Release

`v*`タグのpushだけで起動する。タグから`v`を除いたSemVerと、`cargo metadata`から得た`bettr` package versionが一致しなければ失敗する。

各matrix jobは次を行う。

1. 対象Rust targetを追加する。
2. `cargo build --locked --release --target <target>`する。
3. `bettr --version`を実行し、タグversionを含むことを確認する。
4. `bettr-<version>-<target>.tar.gz`を作り、`bettr`バイナリとLICENSEを含める。
5. archiveごとのSHA-256ファイルを作る。
6. archiveをartifactとしてrelease jobへ渡す。
7. GitHub artifact attestationをarchiveへ付ける。

release jobは全matrix artifactを集め、ソート済みの`SHA256SUMS`を生成し、タグを検証してGitHub Releaseへarchiveとchecksumをuploadする。Release作成に必要な`contents: write`はrelease jobだけに付与し、build jobは`contents: read`、`id-token: write`、`attestations: write`に限定する。

## Asset形式

```text
bettr-0.1.0-x86_64-unknown-linux-gnu.tar.gz
bettr-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
...
SHA256SUMS
```

archive内の実行ファイル名は常に`bettr`とする。macOS/Linuxの利用者は展開後に任意のPATH上へ配置できる。既存のskill installerはGitHub上のskill pathを参照するため、バイナリ配布の導入によって変更しない。

## インストール、upgrade、rollback

READMEにバージョン固定URLを使った手順を記載する。取得したarchiveと`SHA256SUMS`を同じ一時ディレクトリで検証し、展開後のbinaryを一時ファイルへ置いて`bettr --version`を確認してから、既存binaryを`.prev`へ移動して置換する。置換後の起動確認に失敗した場合は`.prev`を元の場所へ戻す。自動installerやPATHの強制変更は追加しない。

## 失敗時の扱い

- version不一致、checksum不一致、archive欠損、binaryの`--version`失敗はReleaseを作成しない。
- build jobの失敗ではrelease jobを実行しない。
- attestationは公開対象archiveごとに作り、検証例として`gh attestation verify`をREADMEに載せる。
- GitHub Releaseの既存tagを上書きする再実行は許可せず、同じversionを再配布する場合は新しいtagまたは手動で既存Releaseを確認する。

## 検証

- YAMLのCI workflowをactionlint相当の構文検査と内容確認で検証する。
- packaging scriptをfixture binaryで実行し、archive内容、checksum、`--version`検証を確認する。
- Rust fmt、clippy、locked test、release buildを実行する。
- READMEの固定版インストール、upgrade、rollback、skill installer導線が実際のasset命名と一致することを差分検査する。
