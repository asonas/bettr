# Idempotency Keys & JSON Batch Updates Design

## Goal

Add durable idempotency to every mutating bettr command and add an atomic
JSON batch API for multiple Issue mutations. A retried successful request
must return the original result without creating another domain event or
audit event; reusing a key for a different operation or payload must fail
explicitly.

## Scope

The change covers:

- the existing mutating commands, including initialization, project and
  Issue writes, coordination writes, decision writes, and Issue state
  transitions;
- an optional global `--idempotency-key` accepted by mutating commands;
- a new `issue batch` command that consumes a JSON array from a file or
  standard input and commits all supported Issue mutations in one SQLite
  transaction;
- database migration, audit projection, revision-conflict and busy-error
  behavior, JSON contract, capability discovery, README, and Codex/Claude
  skill guidance.

Read-only commands do not need an idempotency key. A request without a key
keeps the current behavior.

## Request identity and replay

The request identity is the pair of the operation name and a canonical JSON
payload containing the resolved project, target, expected revision, and all
user-supplied mutation fields. Execution context, generated UUIDs, timestamps,
and output formatting are excluded so a retry from another process can replay
the original result.

The database stores a SHA-256 digest of that canonical payload. The key is
globally unique within the database, so the following cases are distinct:

1. same key, operation, and digest: return the stored successful `data` value;
2. same key with another operation or digest: return
   `idempotency_conflict` with exit code 4;
3. no matching record: execute the mutation and store its result in the same
   transaction as its domain changes and success audit.

Only committed successful operations are memoized. Validation, revision,
domain, and database-busy failures do not create replay records; they keep
their existing error and audit behavior. A replay performs no new domain or
audit write. A rejected key reuse is a normal conflict and is recorded as a
failed attempt.

The `idempotency_records` table stores the key, operation, request digest,
serialized `data` value, and creation time. The audit table gains an optional
idempotency key so the original committed operation can be traced without
exposing the request payload or response body.

## CLI and JSON contract

The global `--idempotency-key KEY` option is available to all commands. Keys
must be nonblank and contain at most 200 Unicode scalar values. The option is
ignored only by read-only commands; supplying it there is harmless and keeps
the global CLI shape consistent.

`issue batch` accepts `--input PATH`; `-` reads standard input. The input is a
JSON array. Each element names one Issue mutation and contains all data needed
for that mutation; an omitted project uses the global project context.
The first implementation supports `issue_create`, `issue_edit`,
`issue_comment`, `issue_start`, `issue_block`, `issue_resume`,
`issue_complete`, `issue_cancel`, and `issue_reopen`. Each operation keeps the
normal revision guard and domain validation of its individual command.

Example input:

```json
[
  {
    "operation": "issue_edit",
    "number": 1,
    "revision": 3,
    "patch": {"title": "Updated title"}
  },
  {
    "operation": "issue_start",
    "number": 2,
    "revision": 1
  }
]
```

The successful response is the normal versioned envelope with `data` as an
ordered array of per-operation results. Replaying the batch returns the same
array. If any item fails, no Issue, domain event, idempotency record, or
success audit from the batch is committed; the command returns the failing
error and records one `issue_batch` failure audit after rollback.

## Transaction boundaries

Individual mutating store methods perform the idempotency lookup before their
existing mutation logic and insert the replay record before committing their
existing domain event and success audit. The lookup and mutation use the same
immediate SQLite transaction, preventing two concurrent requests with one key
from both producing effects.

Batch execution uses one immediate transaction and transaction-local versions
of the Issue create, edit, comment, and transition operations. It emits the
same domain event shapes as the individual commands and one `issue_batch`
success audit. The batch idempotency record is inserted before commit.

The migration advances the SQLite schema from version 3 to version 4. A fresh
database is created directly at version 4; existing databases apply the
migration transactionally.

## Verification

Tests will cover:

- same-key replay for representative create, edit, transition, comment, and
  coordination writes;
- cross-operation and changed-payload key conflicts;
- concurrent same-key requests producing one result, one set of domain events,
  and one success audit;
- revision conflicts and database-busy errors preserving their existing exit
  codes and audit semantics;
- successful and failing batches proving commit and rollback boundaries;
- migration from schema version 3, capability JSON, JSON response shape,
  README/skill contract text, and the complete existing test suite.

