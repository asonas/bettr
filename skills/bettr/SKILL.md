---
name: bettr
description: Use when an agent needs to coordinate work through the local bettr CLI, including Issue recording, claims, leases, human decisions, dependency checks, status supervision, or event-cursor polling.
---

# Bettr Issue Tracking

Use bettr as the local source of truth for agent work. Use the installed CLI and the shared JSON contract; do not invent flags from another tracker.

## Start every agent session

Set the execution context before writes:

```sh
export BETTR_AGENT=codex
export BETTR_SESSION_ID=work-<stable-session-id>
bettr context --json
bettr capabilities --json
```

Use `--project <name>` explicitly when a command accepts an Issue number. Treat an unqualified Issue number as invalid without that project context. Use `command -v bettr` and `bettr --help` to verify the binary; from a source checkout, install with `mise exec -- cargo install --path . --locked`.

Read `.data.capabilities` from `bettr capabilities --json` before selecting a workflow. Use only capabilities whose value is `true`; ignore additive fields and stop with a clear report when the JSON contract version is unsupported. The shared matrix is [../../contracts/capabilities.json](../../contracts/capabilities.json).

The implemented capability names are `issue_dependencies`, `issue_parent`, `issue_claim`, `issue_lease`, `human_decisions`, `event_cursor`, and `capabilities`. Treat `idempotency` as unavailable. Stop when blocked rather than inventing a replacement command.

## Claim and lease workflow

Claim an explicit Issue or let bettr select the first eligible `todo` Issue:

```sh
bettr issue claim 12 --project bettr --json
bettr issue claim --project bettr --json
bettr issue heartbeat 12 --project bettr --json
```

A claim is owned by the `BETTR_AGENT` and `BETTR_SESSION_ID` pair. Heartbeat only renews the lease and does not advance the Issue revision. Keep heartbeats bounded to the expected work interval. A stale lease is not automatically reassigned; use `bettr issue takeover 12 --project bettr --reason "..." --json` only after checking the previous session and recording a concrete reason.

## Record coordination state

Use structured relations when available:

```sh
bettr issue dependency add bettr#3 bettr#12 --json
bettr issue parent set bettr#12 bettr#3 --json
```

When work requires a human choice, create a request and stop agent work on that Issue:

```sh
bettr decision request 12 --project bettr \
  --question "Which behavior is intended?" \
  --background "Both options affect the migration." --json
```

Do not resolve your own decision request. A human or permitted different resolver must use the request UUID:

```sh
BETTR_OPERATOR=reviewer bettr decision resolve <request-uuid> \
  --answer "Use option A" --next-state todo --json
```

An open human decision keeps the Issue in the supervisor's `attention` view and prevents completion. Do not continue by editing around that constraint.

## Revisions, completion, and polling

Fetch the current Issue before every revision-guarded edit or transition:

```sh
bettr issue show 12 --project bettr --json
bettr issue complete 12 --project bettr --revision <current> \
  --summary "Implemented" --verification "mise exec -- cargo test" --json
```

On `revision_conflict`, reread the Issue and inspect the intervening history before deciding whether to retry. Never blindly overwrite. On an unknown write outcome, inspect `issue show`, `issue history`, and `audit list` before repeating the write.

For wayfinder-style polling, persist the last returned cursor and request only newer events:

```sh
bettr event list --after <cursor> --limit 100 --include-issue --json
```

The cursor is exclusive. Consume events in sequence order and advance it only to `next_cursor`; heartbeat renewals, reads, and failures do not appear as domain events.

Use `bettr status --json` for cross-project supervision and stop when an Issue is in `attention` or `stale` until the required human decision or lease handoff is handled. Do not invoke unavailable capabilities, add a daemon, or assume network/authentication behavior.

## Installation

After this repository is available on its default branch, install this skill with the Codex skill installer:

```sh
python3 ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo asonas/bettr --path skills/bettr --ref main
```
