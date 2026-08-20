# JSON and exit-code contract

CLI commands accept `--json`. Successful responses are written to standard output and failures are written to standard error. A process emits one JSON document for the command result.

## Versioning and compatibility

Every response contains `"schema_version": 1` and exactly one of `data` or `error`. A success response never contains `error`, and an error response never contains `data`.

Within schema version 1, new object fields may be added to the envelope, `data`, `error`, `error.details`, and nested objects. Consumers must ignore unknown fields. Existing field names, meanings, and types are stable; removing a field or changing its meaning or type requires a new schema version.

Arrays remain ordered where the command defines an order. Consumers must not infer an order for fields within a JSON object.

## Success envelope

Successful commands exit with code 0 and return:

```json
{
  "schema_version": 1,
  "data": {
    "id": "018f5f89-e8b6-4adb-90fb-0e72b3e49811",
    "project_id": "f77aef3e-6858-4f12-a1dc-785b71de8940",
    "number": 1,
    "title": "Verify the Phase 1 workflow",
    "body": null,
    "state": "todo",
    "priority": "critical",
    "assignee_kind": null,
    "assignee_name": null,
    "revision": 1,
    "created_at": "2026-08-15T09:30:00Z",
    "updated_at": "2026-08-15T09:30:00Z"
  }
}
```

The type of `data` is command-specific. It can be an object, an array, or another JSON value.

## Phase 2 coordination responses

`issue claim`, `issue heartbeat`, and `issue takeover` return an object with `issue` and `lease`. The lease belongs to the `BETTR_AGENT` and `BETTR_SESSION_ID` pair. A heartbeat renews only lease timing; it does not change the Issue revision. An expired lease is shown under `status.data.stale` and is never reassigned automatically.

`decision request` returns a request object with a UUID, `PROJECT#NUMBER` reference, `blocker`, `question`, `options`, `recommendation`, `resume_condition`, background, requester context, and `status: "open"`. `options` contains the viable human choices. Creating the request changes the Issue to `blocked` in the same transaction. `decision resolve` records the answer and resolver context and applies the explicit `next_state`; it cannot select `in_progress` because active work must be entered through an agent claim and lease. Resolve to `blocked` with `--reason` and `--wait-kind`, to `done` with `--summary` and `--verification`, or to `cancelled` with `--reason`; these choices emit the corresponding Issue transition event. Resolve to `todo` and let an agent claim the Issue. An open request is exposed under `status.data.attention`; an Issue with any open request cannot transition to `done`. Older requests created before these fields were added retain empty structured fields and remain readable through their existing question and background.

`issue dependency` and `issue parent` responses use structured `PROJECT#NUMBER` references. An unqualified Issue number requires `--project`. Dependencies are directed `blocks` edges; parent relations are one level deep.

## Event cursor

`event list --after CURSOR [--limit LIMIT] [--include-issue] --json` returns:

```json
{
  "schema_version": 1,
  "data": {
    "next_cursor": 12,
    "has_more": false,
    "events": [
      {
        "sequence": 12,
        "event_type": "issue_completed",
        "project_id": "f77aef3e-6858-4f12-a1dc-785b71de8940",
        "issue_id": "018f5f89-e8b6-4adb-90fb-0e72b3e49811",
        "changed_fields": ["state", "summary", "verification"],
        "revision": 6,
        "created_at": "2026-08-15T09:30:00Z"
      }
    ]
  }
}
```

`after` is exclusive. `next_cursor` remains equal to `after` for an empty page and otherwise equals the last returned sequence. `has_more` indicates that another page exists. Events are ordered by sequence and expose only allowlisted changed-field names and target IDs. Reads, failures, and lease heartbeat renewals are not domain events. With `--include-issue`, the response includes an Issue snapshot read in the same SQLite transaction.

Persist the cursor only after the page has been processed successfully. Consumers must ignore additive fields and stop when `json_contract_version` or the response `schema_version` is unsupported.

## Capability discovery

`capabilities --json` returns the JSON contract version, CLI version, and a boolean capability map. The checked-in source of truth is [`contracts/capabilities.json`](../contracts/capabilities.json). Consumers must invoke only capabilities whose value is `true`; `false` or unknown capabilities are unavailable.

When the `idempotency` capability is `true`, mutating commands accept the
optional global `--idempotency-key KEY`. A successful result is stored with
the operation and canonical request payload. Retrying the same operation and
payload returns the stored result without another domain or success-audit
event. Reusing a key with a different operation or payload returns
`idempotency_conflict`; failed writes are not stored, and callers should keep
the normal `revision_conflict` and `database_busy` retry behavior.

`issue batch --input PATH --json` accepts a JSON array of Issue mutations. The
supported operation tags are `issue_create`, `issue_edit`, `issue_comment`,
`issue_start`, `issue_block`, `issue_resume`, `issue_complete`,
`issue_cancel`, and `issue_reopen`. `--input -` reads stdin. The complete
array runs in one transaction and either commits all operations or rolls all
of them back. A batch idempotency key replays the complete ordered result
array.

## Error envelope

Failed commands return a nonzero exit code and write:

```json
{
  "schema_version": 1,
  "error": {
    "code": "revision_conflict",
    "message": "issue revision conflict; current revision is 6",
    "details": {
      "current_revision": 6
    }
  }
}
```

`error.code` is the stable machine-readable identifier. `error.message` is intended for diagnostics and must not be parsed. `error.details` is present only when the error supplies structured details; Phase 1 revision conflicts include `current_revision`. Idempotency key collisions use `idempotency_conflict` and do not include request-specific details.

## Self-update

`self-update --json` returns `cli`, `codex`, and `claude` component results. Each component includes its `source`, `version`, `revision`, `result`, and installed `path`. `result` is `updated`, `installed`, or `failed`. A failed component also includes `error`; a retained skill backup is reported as `backup`. If any component fails, the command exits 10 and puts the same report in `error.details` with `error.code: "self_update_failed"`.

## Timestamps

All JSON timestamps are RFC 3339 strings in UTC. bettr serializes UTC with the `Z` suffix, for example `2026-08-15T09:30:00Z`. Consumers must parse the timestamp as an instant rather than relying on a fixed number of fractional-second digits.

Human history and audit output renders the same instants in the machine's local timezone. This presentation conversion does not change database or JSON timestamps.

## Audit changed fields

Each `audit list --json` event includes `changed_fields`, an ordered array of allowlisted field names. It never contains field values. Read operations and failed operations return an empty array. For example, an Issue edit that changes a title and priority returns `"changed_fields":["title","priority"]`; transition events name `state` and the applicable metadata fields without exposing reasons, summaries, verification text, Issue bodies, or comment bodies.

## Audit JSONL operations

`audit verify --json` checks the active database-adjacent JSONL file. Use `--path PATH` to verify one archived generation. It validates schema version 1, complete newline-delimited JSON objects, contiguous sequences within the file, unique event IDs, each SHA-256 hash, and each `previous_hash` link. Success data contains `valid`, `event_count`, `first_sequence`, and `last_sequence`.

`audit archive --json` atomically moves a valid active JSONL file to a UTC timestamped generation and creates a new active file. The SQLite cursor is preserved, so the next automatically projected event continues from the archived tail hash. `audit rebuild --json` regenerates the complete active file from SQLite `audit_events` through a temporary verified file, then updates `audit_jsonl_cursor` in the same transaction. A failed rebuild leaves the previous active file in place.

## Redaction

When the `redaction` capability is `true`, the following explicit selectors are available:

```sh
bettr redact issue 12 --project bettr --json
bettr redact comment <comment-uuid> --json
bettr redact audit <audit-event-uuid> --json
```

Redaction requires a human execution context. Issue redaction covers the Issue, related comments and decision text, Issue history metadata, matching idempotency responses/request hashes, and audit metadata for that Issue. Comment redaction covers the selected comment, matching history/idempotency/audit metadata. Audit redaction covers only the selected SQLite audit row. Text values become `[REDACTED]`; JSON metadata retains only safe structural fields and marks the result as redacted. The success data contains only `target_type`, `target_id`, and `changed_count`.

Each redaction mutates SQLite under one `BEGIN IMMEDIATE` transaction and appends a safe success or failure audit event. Repeating a completed redaction succeeds without changing already-safe content. Missing targets return `not_found`; agent execution returns `conflict`. Existing JSONL lines and their hashes are never rewritten because the JSONL projection excludes redacted content and audit metadata. The redaction event is appended on the next normal flush. Existing backups made before redaction are not rewritten and must be treated as sensitive artifacts; retention and automatic deletion are not implemented.

Integrity failures use `error.code: "audit_integrity_failure"` and exit code 10. File-operation failures use `error.code: "audit_operation_failed"` and exit code 10. Integrity error details may contain `line` and `sequence`, plus the fixed recovery instruction `preserve the affected JSONL and run bettr audit rebuild --json`; they never contain raw command arguments or file contents.

## Issue references

Human-facing Issue references use `PROJECT#NUMBER`, for example `bettr#1`. Numbers are positive and allocated independently within each project, so `alpha#1` and `beta#1` identify different Issues. Phase 1 commands pass the parts separately:

```sh
bettr issue show 1 --project bettr --json
```

JSON Issue objects also include the immutable Issue UUID in `id`, the immutable project UUID in `project_id`, and the project-local `number`.

## Exit codes

| Code | Meaning | Representative error code |
| ---: | --- | --- |
| 0 | Success | none |
| 2 | Invalid input, command usage, or unsupported database schema | `invalid_input`, `unsupported_database_schema_version` |
| 3 | Project, Issue, or initialized database not found | `not_found`, `database_not_initialized` |
| 4 | State, name, revision, or idempotency conflict | `invalid_transition`, `project_name_conflict`, `revision_conflict`, `idempotency_conflict` |
| 5 | SQLite remained busy beyond the configured wait | `database_busy` |
| 10 | Internal or audit integrity/operation failure | `internal_error`, `audit_integrity_failure`, `audit_operation_failed` |

When a command that requires an initialized database selects an existing SQLite file without the bettr application ID, the identity preflight rejects it with exit 3:

```json
{
  "schema_version": 1,
  "error": {
    "code": "database_not_initialized",
    "message": "database is not initialized"
  }
}
```

For normal local use, bettr verifies that the selected path resolves to a regular file and checks its header without opening SQLite, then rechecks the identity on the opened connection before enabling connection settings. A known older bettr database schema is migrated in a single transaction before the command continues. This protects an unrelated SQLite database from accidental path selection without changing its existing bytes or creating SQLite sidecars during the header preflight.

The SQLite database schema version is independent from the JSON response `schema_version`. The current database schema version is 7; versions 1 through 6 are migrated automatically and their applied versions are recorded in `schema_migrations`. A bettr database with an unknown schema version is rejected before SQLite is opened for writing and exits 2:

```json
{
  "schema_version": 1,
  "error": {
    "code": "unsupported_database_schema_version",
    "message": "database schema version 99 is unsupported; current version is 7",
    "details": {
      "found_version": 99,
      "current_version": 7
    }
  }
}
```

`init` and `context` do not use the exit 3 contract above. When the selected path already exists, `init --json` exits 2 with `database_already_initialized`. When an unrelated SQLite path is selected, `context --json` returns the resolved context in the success envelope and exits 0 without creating or changing the database.

The preflight is not an adversary-resistant filesystem guarantee: deliberate path replacement by another process during the identity-check/open window is outside the MVP contract.

The success envelope above is the exit-code 0 response from the acceptance fixture's Issue creation. The same fixture yields these input, lookup, and conflict examples:

```sh
# Exit 2: the Issue number must be positive.
bettr issue show 0 --project bettr --json

# Exit 3: no such project-local Issue exists.
bettr issue show 404 --project bettr --json

# Exit 4: revision 5 is stale when the current revision is 6.
bettr issue edit 1 --project bettr --revision 5 --title "Stale edit" --json
```

When another process keeps the fixture database's write lock beyond the configured wait, the write exits 5 with:

```json
{
  "schema_version": 1,
  "error": {
    "code": "database_busy",
    "message": "database is busy"
  }
}
```

Code 5 is a retryable local contention result. If the same fixture database cannot be read safely because of an unexpected internal database failure, the command exits 10 with the same envelope shape and `"code": "internal_error"`. Its safe diagnostic message depends on the failure:

```json
{
  "schema_version": 1,
  "error": {
    "code": "internal_error",
    "message": "database operation failed"
  }
}
```

Code 10 is not a stable diagnosis of the underlying implementation failure; callers should preserve the error code and message for troubleshooting without parsing the message.
