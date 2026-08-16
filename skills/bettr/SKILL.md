---
name: bettr
description: Use when work is tracked in a bettr Issue or a conversation produces a material decision, requirement change, discovery, risk, blocker, next action, or verification result to record.
---

# Bettr Issue Tracking

Treat bettr as the local source of truth for agent work. Use the installed CLI and its JSON contract; never invent flags from another tracker.

## Setup

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
  --body "Implemented the change; verification: mise exec -- cargo test"
```

Save the returned project-local reference (`bettr#1`) and Issue `revision`. Comments are immutable and do not require a revision. Add a new comment to correct an earlier comment.

For edits and state transitions, fetch the current Issue first and pass its revision:

```sh
bettr --project bettr --json issue show 1
bettr --project bettr --json issue edit 1 --revision 1 --title "Updated title"
bettr --project bettr --json issue start 1 --revision 2
bettr --project bettr --json issue block 1 --revision 3 \
  --reason "Waiting for human review" --wait-kind human
bettr --project bettr --json issue complete 1 --revision 4 \
  --summary "Implemented" --verification "mise exec -- cargo test"
```

Use `resume`, `cancel`, or `reopen` only with their required revision and reason fields. Keep the returned revision for the next write.

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

## Handle Failures

- Exit 4 / `revision_conflict`: re-read the Issue, inspect the intervening change, and retry with a deliberate new revision. Never blindly overwrite.
- Exit 3 / `not_found` or `database_not_initialized`: verify `--project`, `bettr context --json`, and the selected database before writing.
- Exit 5 / `database_busy`: report the contention and retry only when the operation is safe to repeat.
- If a write returns an unknown outcome, inspect `issue list`, `issue show`, `issue history`, and `audit list` before creating a duplicate.

Use `bettr status --json` for a cross-project supervisor view and `bettr audit list --json` to verify the recorded operation. Do not put raw command lines or secrets in Issue bodies or comments.

## Install This Skill

After `skills/bettr` is available on the repository's GitHub default branch, install it with the Codex skill installer:

```sh
python3 ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo asonas/bettr --path skills/bettr --ref main
```
