# bettr

bettr is a local, non-interactive issue tracker for work shared by people and agents. It stores projects and Issues in SQLite and provides a CLI, JSON output, and a loopback-only web UI.

## What it can do

| Area | Capabilities |
| --- | --- |
| Projects and Issues | Create, edit, list, show, assign, comment on, and transition Issues across five states: `todo`, `in_progress`, `blocked`, `done`, and `cancelled`. |
| Coordination | Claim Issues, manage leases, send heartbeats, take over stale work, connect Issues with dependencies or one-level parent relations, and link Git worktrees. |
| Human decisions | Ask for a human decision, record the answer, and apply the selected next state. |
| History and supervision | Review comments, Issue history, cross-project status, and audit records. |
| Automation | Use versioned `--json` output, retry-safe idempotency keys, atomic JSON batches, event cursors, and capability discovery. |
| Web UI | Browse projects in a Kanban board, inspect Issue details, follow activity, and resolve existing human decisions. |

## Install

### From source

```sh
cargo install --path .
```

### Release binary

Download a versioned archive for macOS or Linux from [GitHub Releases](https://github.com/asonas/bettr/releases). A versioned download URL has the form `https://github.com/asonas/bettr/releases/download/v<VERSION>/...`.

Verify the archive with its checksum and, when required, its GitHub attestation. Releases include a combined `SHA256SUMS` manifest. For example:

```sh
gh attestation verify bettr-<version>-<target>.tar.gz --repo asonas/bettr
```

Place the extracted `bettr` binary on `PATH`. When upgrading, keep the previous binary as `.prev` until the new version passes its startup check.

## Initialize bettr

Create the local database before using commands that read or write it:

```sh
bettr init
```

The default database locations are:

| Platform | Location |
| --- | --- |
| macOS | `~/Library/Application Support/bettr/bettr.db` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/bettr/bettr.db` |

Use `--database /absolute/path/to/bettr.db` or `BETTR_DATABASE` to select another database. Use `bettr context --json` to inspect the resolved database and project without creating a database.

## Quick start

Create a project and an Issue:

```sh
bettr project create bettr
bettr issue create --project bettr --title "Document the workflow" --priority high
```

Work on the Issue and inspect the result:

```sh
bettr issue start 1 --project bettr --revision 1
bettr issue comment 1 --project bettr --body "Implementation is ready for review"
bettr issue complete 1 --project bettr --revision 2 \
  --summary "Workflow documented" --verification "cargo test passed"
bettr status
```

Edits and state transitions require the current `--revision`. A stale revision fails instead of overwriting a newer update.

## Web UI

Start the local web view:

```sh
bettr web
```

Open the URL printed by bettr, usually `http://127.0.0.1:4242`. Use `--port 0` to select an available port.

The web UI provides a project sidebar, a five-column Kanban board, Issue details, activity, waiting context, dependencies, and decision requests. It is loopback-only and read-focused: it can resolve an existing human decision, while Issue edits, claims, comments, and other changes remain CLI operations.

## Automation and agents

Add `--json` to receive the versioned machine-readable response used by scripts:

```sh
bettr issue show 1 --project bettr --json
bettr status --json
```

Issues may be created without a worktree. Starting or claiming an Issue from a Git worktree attaches the current worktree automatically; additional worktrees can be attached or deactivated explicitly:

```sh
bettr issue worktree add bettr#1 --path /path/to/worktree --json
bettr issue worktree list bettr#1 --json
bettr issue worktree remove bettr#1 --path /path/to/worktree --json
```

Identify an agent and session when coordinating work:

```sh
BETTR_AGENT=codex BETTR_SESSION_ID=session-1 \
  bettr issue claim --project bettr --json
bettr issue heartbeat 1 --project bettr --json
```

Use `--idempotency-key` for retry-safe writes and `issue batch --input <path> --json` for atomic JSON batches. Use `bettr capabilities --json` to discover the available coordination features.

Update the CLI and both agent Skills together from the latest Release:

```sh
bettr self-update
```

Use the current `main` branch instead, or set `update_source = "main"` in the bettr config:

```sh
bettr self-update --source main
```

Remove the installed Codex and Claude Code skills while keeping the CLI:

```sh
bettr self-uninstall
```

The command removes only managed skill directories. Missing skills are reported
as `not_installed`; symlinks and directories without the expected skill
manifest are left untouched.

The JSON response contract is documented in [`docs/json-contract.md`](docs/json-contract.md). Agent-specific workflows are documented in [`skills/bettr`](skills/bettr) and [`skills/bettr-claude`](skills/bettr-claude).

## Repository project context

This repository is explicitly bound to the `bettr` project by the root
`.bettr.toml`. Run `bettr context --json` from the repository or any nested
directory and confirm that `project.value` is `bettr` and `project.source` is
`directory_config` before reading or writing Issues.

When a directory has no project configuration and `project.value` is null,
ask the human for the project instead of inferring it from `bettr status`, an
Issue's priority, update time, assignee, or the repository name. `bettr status`
remains a cross-project supervision view and is not a project-selection input.

## Data and limits

- bettr stores data locally in SQLite.
- bettr does not start agents, share data over a network, or use external databases.
- The web UI binds only to the loopback interface.
- JSONL audit export, redaction, and SQLite backup/restore are available capabilities; retention and `doctor` diagnostics remain future work. See [`contracts/capabilities.json`](contracts/capabilities.json) for the machine-readable availability matrix.

### SQLite backup and restore

Create a snapshot to an explicit, new path:

```sh
bettr --database /path/to/bettr.db backup --output /path/to/snapshot.db --json
```

The snapshot is a single SQLite file produced with SQLite's online backup API. It includes committed WAL content and never copies `-wal`, `-shm`, or `-journal` sidecars. Existing output is rejected. Restore requires an explicit input and output plus confirmation:

```sh
bettr restore --input /path/to/snapshot.db \
  --output /path/to/restored.db --yes --json
```

Before publishing, restore validates SQLite identity, supported schema version, integrity, and foreign keys, stages the destination, and rebuilds its active `.audit.jsonl` from SQLite audit events. Existing database or sidecar files require both `--replace` and `--yes`; archived JSONL generations are not bundled or restored. Errors use stable codes such as `invalid_backup`, `backup_output_exists`, `confirmation_required`, and `backup_operation_failed` without echoing filesystem details.

## License

bettr is available under the [MIT License](LICENSE).
