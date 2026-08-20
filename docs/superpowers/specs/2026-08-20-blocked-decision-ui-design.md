# Blocked Reasons and Human Decision Resolution Web UI

> Status: approved design, implementation in progress

## Goal

Make blocked and attention Issues understandable in the existing local Web UI and provide a safe browser path for a human to resolve an existing `human_decisions` request without changing bettr's state or transition rules.

## Existing contracts

- `IssueState` and `Transition` remain the only source of truth for state transitions.
- `App::resolve_decision` and `Database::resolve_decision` remain the mutation path for decision resolution. The Web route does not write `issues`, `decision_requests`, `domain_events`, or `audit_events` directly.
- A decision request is resolved only with a human execution context. The requesting agent session cannot resolve its own request.
- Resolution accepts only `todo`, `blocked`, `done`, or `cancelled`. `in_progress` is not a decision-resolution target.
- Required metadata remains state-specific: no metadata for `todo`; `reason` and `wait_kind` for `blocked`; `summary` and `verification` for `done`; `reason` for `cancelled`.
- The active explicit database reports `idempotency: false`. This feature therefore does not add or depend on idempotency records or a new retry protocol. The browser prevents duplicate in-flight submissions, and an outcome that is not known to the browser requires reloading the Issue before another submission.
- JSONL audit work associated with Issue #9 is out of scope.

## User flow

1. A human opens the loopback Web UI.
2. Status and Issue lists show attention/blocked context in words: what is waiting, why it is waiting, the wait kind, and the number and questions of unresolved decisions.
3. Issue detail shows the current revision, waiting context, dependencies, all decision requests, requester and creation time, and an answer form for each open request.
4. The human selects an answer and a valid next state. The form exposes and validates only the metadata required by that state.
5. The browser sends the displayed revision and disables that form while the request is in flight.
6. The server validates the request, checks the revision inside the existing SQLite immediate transaction, and calls the existing decision-resolution path.
7. On success the browser reloads the detail and polls status so the state, decision count, activity, resolver, and audit-backed history are visible.

## Read model

The existing `Issue` and CLI JSON shapes are not changed. Web responses add a Web-specific projection to status/list items:

```json
{
  "project": "bettr",
  "number": 1,
  "title": "Choose a deployment target",
  "state": "blocked",
  "revision": 4,
  "wait": {
    "kind": "human",
    "label": "Human decision",
    "reason": "A human decision is required"
  },
  "unresolved_decision_count": 2,
  "decision_questions": [
    "Which deployment target should be used?",
    "Should compatibility mode stay enabled?"
  ]
}
```

`wait` is omitted when no waiting context applies. `kind` uses an existing `WaitKind` value when one is recorded; `label` and `reason` are always human-readable. The projection gives priority to open human decisions, then the latest `issue_blocked` event metadata, then dependency context, and finally an explicit “waiting details are not recorded” message. It never presents an enum or UUID as the only explanation.

The Issue detail response adds:

```json
{
  "project": "bettr",
  "issue": {},
  "history": [],
  "decisions": [],
  "dependencies": [],
  "wait": {}
}
```

`decisions` contains the existing `DecisionRequest` values, including open and resolved requests. Only open requests receive an answer form. `dependencies` contains the existing `IssueDependency` values and is display-only.

The read path obtains decision requests through a new audited read operation backed by the existing `decision_requests` table. Blocked reason and wait kind are derived from existing `domain_events` metadata; no new Issue columns or parallel status table are introduced.

## Resolve API

The server adds one mutation route:

```text
POST /api/decisions/<request-uuid>/resolve
Content-Type: application/json
```

The body is a bounded, unknown-field-rejecting JSON object:

```json
{
  "expected_revision": 4,
  "answer": "Use the staged deployment target.",
  "next_state": "blocked",
  "reason": "Waiting for the rollout window.",
  "wait_kind": "external"
}
```

`expected_revision` is required and must be positive. The other fields map one-to-one to `DecisionResolutionInput`. The Web layer validates the positive revision and JSON shape; the existing domain validation enforces answer and state-specific metadata.

Internally, the resolution store method accepts an optional expected revision. Existing CLI callers pass no guard and retain their current CLI contract. The Web caller passes the displayed revision. Inside the existing immediate transaction, the current Issue is loaded and compared before the Issue update. A mismatch returns the existing `revision_conflict` error and no decision, state, event, or audit mutation is committed.

Success returns the existing resolved `DecisionRequest` in the versioned JSON envelope. The browser then performs a fresh detail and status read rather than constructing a local state transition.

## Error behavior

The HTTP layer preserves the existing `AppError` code and status mapping:

| Condition | HTTP status | UI behavior |
| --- | ---: | --- |
| Invalid JSON, missing fields, blank answer, invalid next state | 400 | Keep the form, identify the fields to correct, and announce the error. |
| Issue or request UUID not found | 404 | Show that the decision is no longer available and offer reload. |
| Human-context/permission conflict, already-resolved request, invalid transition, multiple open requests preventing `done` | 409 | Show the server's human-readable conflict and offer reload. |
| Displayed revision is stale | 409 | Tell the human that the Issue changed and require reload before another submit. |
| SQLite busy/locked | 503 | Keep the answer, report temporary database contention, and require an explicit retry after reload. |
| Unexpected server/database failure | 500 | Show a non-technical error and retain the detail route. |

The UI never silently retries a POST. The submit button is disabled for the specific form during the request, and a second submit event is ignored while it is pending. If the response outcome is unknown, the form does not automatically resend the mutation.

## Network and authority boundary

- `TcpListener` continues to bind only to `127.0.0.1`; the API is not reachable through an external interface.
- No authentication, multi-user authorization, CORS allowance, or external bind is added in this scope. This is intentionally a local, unauthenticated UI, and external unauthenticated use is not permitted by the bind boundary.
- The only Web write is the decision resolve route. There is no generic SQL endpoint, CLI subprocess invocation, or arbitrary Issue mutation endpoint.
- A process started with an agent execution context still fails the existing human-only resolution rule; the Web route cannot turn an agent into a permitted resolver.

## Frontend behavior and accessibility

- Every open request has a labelled `Answer` textarea, a labelled next-state select, state-specific labelled inputs, a clear submit button, and a live feedback region.
- Changing next state toggles the relevant inputs and their `required` constraints. Hidden fields are not submitted.
- Multiple open requests render as separate cards/forms. A blank decision collection renders an explicit empty state; the Issue activity collection retains its existing empty state.
- Waiting meaning is conveyed by text and labels, not color alone. Raw UUIDs are not used as the visible question or reason.
- Forms remain keyboard-operable, have visible focus styles, use semantic fieldsets/headings, and announce validation, conflict, busy, and success feedback with `role="alert"` or `aria-live`.
- Reduced-motion behavior and the loopback/local-database indicator remain intact.

## Testing strategy

Rust tests will cover:

- decision request reads in the detail and enriched status/list responses;
- human-readable wait projection for human, dependency, external, and missing metadata cases;
- successful Web resolution and refresh-visible state/decision data;
- state-specific required fields and rejection of `in_progress`;
- stale revision rejection without a partial write;
- multiple open requests blocking a `done` resolution while allowing the existing permitted resolutions;
- agent/non-human resolution rejection, invalid UUID/input, already-resolved requests, and not-found behavior;
- SQLite busy mapping to 503;
- loopback startup and rejection of non-GET routes except the exact resolve POST route.

Frontend tests will cover:

- blocked/attention reason, wait label, decision count, and question rendering in list/detail views;
- multiple forms and empty decision state;
- dynamic required fields for all four valid next states;
- disabled/in-flight double-submit protection;
- conflict, revision, permission, validation, and busy messages with reload behavior;
- successful resolve followed by detail/status refresh;
- labels, live regions, keyboard-accessible controls, and focus-visible markup.

## Out of scope

- Agent self-resolution or any new resolver authority.
- A Web-specific state machine, transition semantics, or direct SQLite writes.
- External exposure, authentication, multi-user roles, notifications, or background jobs.
- JSONL audit export or any Issue #9 audit redesign.
- General Web mutation endpoints beyond human decision resolution.
