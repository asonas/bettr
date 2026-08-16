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

### Database safety boundary

For normal local use, bettr protects unrelated SQLite databases from accidental selection by commands that require an initialized database. Before those commands ask SQLite to open a path, bettr verifies that it resolves to a regular file, then reads its header without modifying it and checks the bettr application ID and a supported schema version. Older supported bettr schemas are migrated in a single transaction. After opening the database, bettr checks the identity again on that same connection before enabling connection settings such as WAL. If the identity preflight rejects an unrelated path, these commands return exit code 3 with `database_not_initialized`; an unknown bettr schema version returns exit code 2 with `unsupported_database_schema_version`. Existing file bytes are not changed and bettr does not create `-wal`, `-shm`, or `-journal` sidecars during the header preflight.

`bettr init` and `bettr context` have separate contracts. `init` refuses any existing path with exit code 2 and `database_already_initialized`. For an unrelated SQLite path, `context` reports the resolved path without requiring a valid bettr database and exits 0; it does not create or change the database.

This is an MVP safeguard against accidental selection, not a complete defense against a hostile local process. Deliberate replacement of the selected path during the identity-check/open window is outside the MVP guarantee.

## Shortest workflow

Create a project and an Issue:

```sh
bettr project create bettr
bettr issue create --project bettr --title "Document Phase 1" --priority critical
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

## Codex skill

The repository includes a `bettr` skill for agents that need to record work in the local Issue tracker. Once this repository is available on GitHub, install it with the Codex skill installer:

```sh
python3 ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo asonas/bettr --path skills/bettr --ref main
```

## Phase 1 scope

Phase 1 provides projects, priorities (`critical`, `high`, `medium`, and `low`), five Issue states (`todo`, `in_progress`, `blocked`, `done`, and `cancelled`), immutable comments, revision-guarded edits, history, cross-project status, execution context, and SQLite audit events.

The Phase 1 CLI does not start agents, share data over a network, or support external databases. It also does not yet provide claim or lease coordination, decision requests, dependencies, parent-child Issues, structured references, event cursors, idempotency keys, JSONL audit export, backup and restore, redaction, or `doctor` diagnostics.

## Performance baseline

Run the release-mode child-process latency baseline with:

```sh
cargo bench --bench cli_latency
```

The harness reports p50 and p95 for 1,000 `issue show` operations, 1,000 revision-guarded `issue edit` operations, and 50 selective `issue list` operations over 100,000 Issues. The large fixture starts from the CLI-initialized production schema and identity, then uses a deterministic bulk transaction for practical setup time. It reports measurements without enforcing the 50 ms or 200 ms targets in the normal test suite. Record machine information and results in release notes before publishing a performance claim.

## License

bettr is available under the [MIT License](LICENSE).
