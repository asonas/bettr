# bettr

bettr is a local, non-interactive issue tracker for work shared by people and agents. It stores projects and Issues in SQLite and provides a CLI, JSON output, and a loopback-only web UI.

## What it can do

| Area | Capabilities |
| --- | --- |
| Projects and Issues | Create, edit, list, show, assign, comment on, and transition Issues across five states: `todo`, `in_progress`, `blocked`, `done`, and `cancelled`. |
| Coordination | Claim Issues, manage leases, send heartbeats, take over stale work, and connect Issues with dependencies or one-level parent relations. |
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

The JSON response contract is documented in [`docs/json-contract.md`](docs/json-contract.md). Agent-specific workflows are documented in [`skills/bettr`](skills/bettr) and [`skills/bettr-claude`](skills/bettr-claude).

## Data and limits

- bettr stores data locally in SQLite.
- bettr does not start agents, share data over a network, or use external databases.
- The web UI binds only to the loopback interface.
- JSONL audit export and redaction are available capabilities; backup and restore,
  retention, and `doctor` diagnostics remain future work. See
  [`contracts/capabilities.json`](contracts/capabilities.json) for the
  machine-readable availability matrix.

## License

bettr is available under the [MIT License](LICENSE).
