# bettr

bettr is a local, non-interactive Issue tracker for agent work. It stores projects, Issues, comments, state transitions, execution context, and audit events in SQLite. Phase 1 supports macOS and Linux.

## Install

Install the current checkout with Cargo:

```sh
cargo install --path .
```

Initialize the database explicitly before using it:

```sh
bettr init
```

By default, bettr stores data at:

- macOS: `~/Library/Application Support/bettr/bettr.db`
- Linux: `${XDG_DATA_HOME:-~/.local/share}/bettr/bettr.db`

Use `--database /absolute/path/to/bettr.db` or `BETTR_DATABASE` to select another SQLite file. Run `bettr context --json` to inspect the resolved database and project without creating a database.

## Shortest workflow

Create a project and an Issue:

```sh
bettr project create bettr
bettr issue create --project bettr --title "Document Phase 1" --priority high
```

The new Issue is `bettr#1` at revision 1. Assignment is an optimistic, revision-guarded edit; every successful edit or state transition returns the next revision:

```sh
bettr issue edit 1 --project bettr --revision 1 \
  --assignee-kind agent --assignee-name codex
bettr issue start 1 --project bettr --revision 2
bettr issue comment 1 --project bettr --body "Implementation is ready for review"
bettr issue block 1 --project bettr --revision 3 \
  --reason "Waiting for review" --wait-kind human
bettr issue resume 1 --project bettr --revision 4
bettr issue complete 1 --project bettr --revision 5 \
  --summary "Phase 1 documented" --verification "cargo test passed"
```

Inspect the result across all projects or review the Issue history:

```sh
bettr status
bettr issue history 1 --project bettr
bettr audit list
```

Use the revision returned by the previous write. A stale revision fails with exit code 4 instead of overwriting another process's update.

## JSON and execution context

Add `--json` to any command to receive the versioned machine-readable envelope described in [the JSON contract](docs/json-contract.md):

```sh
bettr issue show 1 --project bettr --json
bettr status --json
```

Agent callers should identify themselves through the environment. `BETTR_SESSION_ID` is optional when `BETTR_AGENT` is set:

```sh
BETTR_AGENT=codex BETTR_SESSION_ID=session-1 \
  bettr issue comment 1 --project bettr --body "Tests pass" --json
```

Human automation can set `BETTR_OPERATOR`; otherwise bettr uses the OS username:

```sh
BETTR_OPERATOR=reviewer bettr issue resume 1 --project bettr --revision 4
```

`BETTR_PROJECT` supplies a default project. Project resolution is command argument, environment, nearest `.bettr.toml`, user config, then no default. User config is stored at `~/Library/Application Support/bettr/config.toml` on macOS and `${XDG_CONFIG_HOME:-~/.config}/bettr/config.toml` on Linux.

## Phase 1 scope

Phase 1 provides projects, five Issue states (`todo`, `in_progress`, `blocked`, `done`, and `cancelled`), immutable comments, revision-guarded edits, history, cross-project status, execution context, and SQLite audit events.

The Phase 1 CLI does not start agents, share data over a network, or support external databases. It also does not yet provide claim or lease coordination, decision requests, dependencies, parent-child Issues, structured references, event cursors, idempotency keys, JSONL audit export, backup and restore, redaction, or `doctor` diagnostics.

## Performance baseline

Run the release-mode child-process latency baseline with:

```sh
cargo bench --bench cli_latency
```

The harness reports p50 and p95 for 1,000 `issue show` operations and 1,000 revision-guarded `issue edit` operations against one prepared database. It reports measurements without enforcing the 50 ms target in the normal test suite. Record machine information and results in release notes before publishing a performance claim.

## License

bettr is available under the [MIT License](LICENSE).
