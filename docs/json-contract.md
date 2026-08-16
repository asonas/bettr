# JSON and exit-code contract

Phase 1 commands accept `--json`. Successful responses are written to standard output and failures are written to standard error. A process emits one JSON document for the command result.

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

`error.code` is the stable machine-readable identifier. `error.message` is intended for diagnostics and must not be parsed. `error.details` is present only when the error supplies structured details; Phase 1 revision conflicts include `current_revision`.

## Timestamps

All JSON timestamps are RFC 3339 strings in UTC. bettr serializes UTC with the `Z` suffix, for example `2026-08-15T09:30:00Z`. Consumers must parse the timestamp as an instant rather than relying on a fixed number of fractional-second digits.

Human history and audit output renders the same instants in the machine's local timezone. This presentation conversion does not change database or JSON timestamps.

## Audit changed fields

Each `audit list --json` event includes `changed_fields`, an ordered array of allowlisted field names. It never contains field values. Read operations and failed operations return an empty array. For example, an Issue edit that changes a title and priority returns `"changed_fields":["title","priority"]`; transition events name `state` and the applicable metadata fields without exposing reasons, summaries, verification text, Issue bodies, or comment bodies.

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
| 2 | Invalid input or command usage | `invalid_input` |
| 3 | Project, Issue, or initialized database not found | `not_found`, `database_not_initialized` |
| 4 | State, name, or revision conflict | `invalid_transition`, `project_name_conflict`, `revision_conflict` |
| 5 | SQLite remained busy beyond the configured wait | `database_busy` |
| 10 | Internal failure | `internal_error` |

Selecting an existing SQLite file that does not have the current bettr application ID and schema version exits 3 with:

```json
{
  "schema_version": 1,
  "error": {
    "code": "database_not_initialized",
    "message": "database is not initialized"
  }
}
```

For normal local use, bettr checks the file header without opening SQLite, then rechecks the identity on the opened connection before enabling connection settings. This protects an unrelated SQLite database from accidental path selection without changing its existing bytes or creating SQLite sidecars during the header preflight. It is not an adversary-resistant filesystem guarantee: deliberate path replacement by another process during the identity-check/open window is outside the MVP contract.

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
