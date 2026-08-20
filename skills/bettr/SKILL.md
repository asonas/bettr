---
name: bettr
description: Use when work is tracked in a bettr Issue or when an agent needs to coordinate work through the local bettr CLI, including Issue recording, claims, leases, human decisions, dependency checks, status supervision, or event-cursor polling.
---

# Bettr Issue Tracking

Use bettr as the local source of truth for agent work. Use the installed CLI and the shared JSON contract; do not invent flags from another tracker.

## Start every agent session

1. At the first bettr command in a session, and after a command-not-found or unknown command/option error, verify the binary:

   ```sh
   command -v bettr
   bettr --help
   ```

   If this checkout is the Rust source and the binary is missing, install it with `mise exec -- cargo install --path . --locked`, then verify `command -v bettr` again.

2. At the start of a bettr session, resolve the database without changing it:

   ```sh
   bettr context --json
   ```

   Run `bettr init` only when creating a new database. Do not use it on an existing path.

3. For agent writes, set execution context in the environment. These are not CLI flags:

   ```sh
   export BETTR_AGENT=codex
   export BETTR_SESSION_ID=work-<stable-session-id>
   ```

   Human operations use `BETTR_OPERATOR`; otherwise bettr records the OS username.

## Record Work

Use `--project <name>` explicitly and `--json` for agent calls. Successful responses have a `data` field and `schema_version: 1`.

```sh
bettr --project bettr --json issue create \
  --title "Implement the next bettr milestone" \
  --body "Describe the goal and acceptance criteria." \
  --priority high

bettr --project bettr --json issue comment 1 \
  --body "Implemented the change; verification is available"
```

Save the returned project-local reference (`bettr#1`) and Issue `revision`. Comments are immutable and do not require a revision. Add a new comment to correct an earlier comment.

For edits and state transitions, fetch the current Issue first and pass its revision. Treat an unqualified Issue number as invalid without explicit project context.

## Capability discovery

Read `.data.capabilities` from `bettr capabilities --json` before selecting a workflow. Use only capabilities whose value is `true`; ignore additive fields and stop with a clear report when the JSON contract version is unsupported. The shared matrix is [../../contracts/capabilities.json](../../contracts/capabilities.json).

The implemented capability names are `issue_dependencies`, `issue_worktrees`, `issue_parent`, `issue_claim`, `issue_lease`, `human_decisions`, `event_cursor`, `capabilities`, `idempotency`, `audit_jsonl`, and `redaction`.

For retry-safe writes, pass the optional global `--idempotency-key <key>`. The same key is replayed only when the operation and canonical request payload match; a mismatch returns `idempotency_conflict` with exit code 4. Failed writes are not memoized, and normal `revision_conflict` and `database_busy` behavior remains unchanged.

Use `bettr issue batch --input <path> --json` for an atomic JSON array of `issue_create`, `issue_edit`, `issue_comment`, `issue_start`, `issue_block`, `issue_resume`, `issue_complete`, `issue_cancel`, and `issue_reopen` operations. Use `--input -` for stdin. A batch commits all operations or rolls them all back; `--idempotency-key` replays the complete result.

When `audit_jsonl` is enabled, every CLI invocation automatically projects its SQLite audit events to the database-adjacent `.audit.jsonl` file. This JSONL projection preserves the SQLite event cursor, chains each line with a SHA-256 hash, serializes concurrent appenders through SQLite, and retries unwritten events on the next invocation. It emits only safe operation/context/result fields; never add raw argv, Issue or comment bodies, or secrets to the log. Issue #10 provides `audit verify [--path PATH]`, `audit archive`, and `audit rebuild`. When `redaction` is enabled, use human-only `redact issue`, `redact comment`, or `redact audit` with an explicit target; it replaces SQLite content in one transaction, leaves existing JSONL and backups unchanged, and does not implement automatic retention.

Stop when blocked by an unsupported contract or required human decision rather than inventing a replacement command.

## Claim and lease workflow

Claim an explicit Issue or let bettr select the first eligible `todo` Issue:

```sh
bettr issue claim 12 --project bettr --json
bettr issue claim --project bettr --json
bettr issue heartbeat 12 --project bettr --json
```

A claim is owned by the `BETTR_AGENT` and `BETTR_SESSION_ID` pair. Heartbeat only renews the lease and does not advance the Issue revision. Keep heartbeats bounded to the expected work interval. A stale lease is not automatically reassigned; use `bettr issue takeover 12 --project bettr --reason "..." --json` only after checking the previous session and recording a concrete reason.

## Status and Conversation Updates

When the conversation explicitly identifies an active Issue as `project#number`, review its status and conversation updates before finalizing each response. Keep that Issue active until another is explicitly identified; never infer one from project defaults, recent Issues, or repository state. A new conversation requires an explicit Issue reference.

Use these automatic status transitions only when evidence is unambiguous:

- `todo` to `in_progress` after implementation or investigation actually starts; design discussion alone is not enough.
- `in_progress` to `blocked` after a concrete blocker and waiting reason are identified.
- `in_progress` to `done` only after the requested scope is complete and verification evidence is available.
- Never transition automatically to `cancelled`.

Add one comment only for a changed purpose, acceptance criterion, implementation approach, risk, blocker, next action, or verification result. Skip questions, hypotheses, plans, repeated information, unrelated discussion, ambiguous targets, and Issues already `done` or `cancelled`. Do not create Issues or change other Issue fields automatically. An explicit user opt-out takes precedence.

When a comment or status transition is appropriate:

1. Show the current Issue and use its latest `revision`.
2. Inspect recent history only for a comment candidate; skip duplicates.
3. Add the comment before transitioning the Issue:

   ```markdown
   [Conversation update]

   Decision / Discovery:
   Impact:
   Next:
   ```

4. Apply the transition with required metadata, verify it, and tell the user what changed.

If an update fails or its result is unknown, inspect Issue history and the audit log before retrying; never retry blindly. If no active Issue is identified, do not create or infer one. Mention a possible update only when useful.

## Record coordination state

Use structured relations when available:

```sh
bettr issue dependency add bettr#3 bettr#12 --json
bettr issue parent set bettr#12 bettr#3 --json
```

When work requires a human choice, create exactly one request for that choice and stop agent work on that Issue. Before invoking the command, write the Issue-facing context in this order:

```text
Blocker: what cannot proceed
Human decision: the single question the human must answer
Options: every viable choice and its consequence
Recommendation: the agent's recommended choice and why
Resume condition: what becomes true after the answer
```

Store each field in the decision request so the Issue detail can show the blocker and the human action without requiring the reader to reconstruct the agent's reasoning:

```sh
bettr decision request 12 --project bettr \
  --blocker "The migration cannot proceed until the storage format is selected." \
  --question "Which behavior is intended?" \
  --option "Use the existing format: no migration, but the new field is unavailable." \
  --option "Add a migration: the new field is available, but old data is rewritten." \
  --recommendation "Add a migration because the feature requires the new field." \
  --resume-condition "The selected format is recorded and the migration contract is fixed." \
  --background "Both options affect the migration." --json
```

Do not resolve your own decision request. A human or permitted different resolver must use the request UUID:

```sh
BETTR_OPERATOR=reviewer bettr decision resolve <request-uuid> \
  --answer "Use option A" --next-state todo --json
```

When resolving directly to `blocked`, provide `--reason` and `--wait-kind`; when resolving to `done`, provide `--summary` and `--verification`; when resolving to `cancelled`, provide `--reason`. These resolutions emit the matching Issue transition event. Use `todo` when the agent should claim the Issue again.

An open human decision keeps the Issue in the supervisor's `attention` view and prevents completion. Do not continue by editing around that constraint.

Resolve decisions to `todo`, `blocked`, `done`, or `cancelled`; do not select `in_progress`, because active work must be re-entered through an agent claim and lease.

## Revisions, completion, and polling

Fetch the current Issue before every revision-guarded edit or transition:

```sh
bettr issue show 12 --project bettr --json
bettr issue complete 12 --project bettr --revision <current> \
  --summary "Implemented" --verification "mise exec -- cargo test" --json
```

On `revision_conflict`, reread the Issue and inspect the intervening history before deciding whether to retry. Never blindly overwrite. On an unknown write outcome, inspect `issue show`, `issue history`, and `audit list` before repeating the write.

## Handle Failures

- Exit 4 / `revision_conflict`: re-read the Issue, inspect the intervening change, and retry only with a deliberate new revision.
- Exit 3 / `not_found` or `database_not_initialized`: verify `--project`, `bettr context --json`, and the selected database before writing.
- Exit 5 / `database_busy`: report the contention and retry only when the operation is safe to repeat.
- If a write returns an unknown outcome, inspect `issue list`, `issue show`, `issue history`, and `audit list` before creating a duplicate.

Use `bettr status --json` for a cross-project supervisor view and `bettr audit list --json` to verify the recorded operation. Do not put raw command lines or secrets in Issue bodies or comments.

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
