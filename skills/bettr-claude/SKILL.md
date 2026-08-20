---
name: bettr-claude
description: Use when Claude Code needs to coordinate local agent work through bettr, including Issue claims, leases, human decisions, dependencies, status supervision, and event-cursor polling.
---

# Bettr for Claude Code

Use the same bettr CLI contract as the Codex adapter. Keep the runtime adapter thin: the database, JSON envelope, revision rules, lease ownership, and capability names are shared with [the capability matrix](../../contracts/capabilities.json).

## Session setup

Before agent writes, set an explicit identity and discover the installed contract:

```sh
export BETTR_AGENT=claude
export BETTR_SESSION_ID=work-<stable-session-id>
bettr context --json
bettr capabilities --json
```

Verify `command -v bettr` and `bettr --help`. Read `.data.capabilities`, reject unsupported JSON contract versions, and invoke only capabilities marked `true`. Treat unknown capability names as unavailable; additive response fields may be ignored.

The implemented capability names are `issue_dependencies`, `issue_parent`, `issue_claim`, `issue_lease`, `human_decisions`, `event_cursor`, `capabilities`, and `idempotency`.

For retry-safe writes, pass the optional global `--idempotency-key <key>`. The same key is replayed only when the operation and canonical request payload match; a mismatch returns `idempotency_conflict` with exit code 4. Failed writes are not memoized, and normal `revision_conflict` and `database_busy` behavior remains unchanged.

Use `bettr issue batch --input <path> --json` for an atomic JSON array of `issue_create`, `issue_edit`, `issue_comment`, `issue_start`, `issue_block`, `issue_resume`, `issue_complete`, `issue_cancel`, and `issue_reopen` operations. Use `--input -` for stdin. A batch commits all operations or rolls them all back; `--idempotency-key` replays the complete result.

Stop when blocked by an unsupported contract or required human decision rather than inventing a replacement command.

## Work lifecycle

Claim an eligible Issue and renew only the current session's lease:

```sh
bettr issue claim --project bettr --json
bettr issue claim 12 --project bettr --json
bettr issue heartbeat 12 --project bettr --json
```

The lease is owned by the `BETTR_AGENT` plus `BETTR_SESSION_ID` pair. Heartbeat does not change the Issue revision. A stale lease remains `in_progress`; investigate it before using a reasoned takeover:

```sh
bettr issue takeover 12 --project bettr --reason "Previous session expired after verification" --json
```

Use `issue_dependencies` and `issue_parent` for structured coordination rather than encoding relationships in prose. When a dependency or review choice blocks progress, preserve the relation and report it in the Issue.

## Human decisions

Create exactly one request for the human choice and stop work on that Issue until it is resolved. Make the Issue-facing context explicit in this order:

```text
Blocker: what cannot proceed
Human decision: the single question the human must answer
Options: every viable choice and its consequence
Recommendation: the agent's recommended choice and why
Resume condition: what becomes true after the answer
```

Store each field in the request so the Issue detail exposes the blocker and the required human action:

```sh
bettr decision request 12 --project bettr \
  --blocker "The migration cannot proceed until the storage format is selected." \
  --question "Which behavior is intended?" \
  --option "Use the existing format: no migration, but the new field is unavailable." \
  --option "Add a migration: the new field is available, but old data is rewritten." \
  --recommendation "Add a migration because the feature requires the new field." \
  --resume-condition "The selected format is recorded and the migration contract is fixed." \
  --background "The choice changes the rollout." --json
```

Do not resolve the requesting agent session's own request. A human resolver can record the answer and explicit next state:

```sh
BETTR_OPERATOR=reviewer bettr decision resolve <request-uuid> \
  --answer "Use the safer behavior" --next-state todo --json
```

When resolving directly to `blocked`, provide `--reason` and `--wait-kind`; when resolving to `done`, provide `--summary` and `--verification`; when resolving to `cancelled`, provide `--reason`. These resolutions emit the matching Issue transition event. Use `todo` when the agent should claim the Issue again.

Open requests appear in `status` as `attention` and block completion. Do not bypass them by writing state directly.

Resolve decisions to `todo`, `blocked`, `done`, or `cancelled`; do not select `in_progress`, because active work must be re-entered through an agent claim and lease.

## Revision conflicts and event polling

Read the Issue immediately before a revision-guarded write. If a write returns a revision conflict, reread and reconcile the intervening change; never retry the old revision blindly. If a write's outcome is unknown, inspect `issue show`, `issue history`, and `audit list` before repeating it.

Persist the event cursor and poll exclusively after the last consumed sequence:

```sh
bettr event list --after <cursor> --limit 100 --include-issue --json
```

Consume events in sequence order and advance only to `next_cursor`. `event_cursor` omits reads, failures, and lease heartbeat renewals. Use `status --json` to supervise `attention`, `stale`, `blocked`, and active work. Do not assume unavailable capabilities, a daemon, a network service, or authentication.

## Claude Code installation

Load this directory as the Claude Code adapter for the project. Keep its command semantics synchronized with `skills/bettr/SKILL.md`; runtime-specific installation details belong in [README.md](README.md), while this file contains the operational contract.
