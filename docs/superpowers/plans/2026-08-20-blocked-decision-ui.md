# Blocked Reasons and Human Decision Resolution Web UI Implementation Plan

> Status: implementation and verification complete

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the existing loopback Web UI with human-readable blocked/attention context and a safe browser workflow for resolving existing human decision requests.

**Architecture:** Add an audited decision-request read method and an optional expected-revision guard to the existing App/SQLite decision path. Build a Web-only projection from existing Issue, event, decision, and dependency data, expose one bounded JSON POST route for resolution, and render state-specific accessible forms in the embedded Vanilla JavaScript UI. The active database's `idempotency: false` capability remains respected; duplicate in-flight submissions are prevented in the browser and unknown outcomes require reload.

**Tech Stack:** Rust 2024, `std::net::TcpListener`, rusqlite, Serde, existing `App`/`Database` contracts, embedded HTML/CSS/Vanilla JavaScript, Vitest/jsdom, Rust integration tests.

**Spec:** `docs/superpowers/specs/2026-08-20-blocked-decision-ui-design.md`

## Global Constraints

- Keep `IssueState`, `Transition`, `DecisionResolutionInput`, human execution context, and existing audit/domain-event writes authoritative.
- Add no Issue columns, Web-specific state transitions, generic SQL endpoint, CLI subprocess path, external bind, authentication system, or JSONL audit implementation.
- Bind only to `127.0.0.1`; expose only the exact decision resolve POST route as a Web write.
- Preserve JSON envelope `schema_version: 1` and existing `AppError` codes/status mapping.
- The Web resolve body must require a positive displayed revision and reject unknown JSON fields.
- Valid next states are exactly `todo`, `blocked`, `done`, and `cancelled`; state-specific metadata requirements remain unchanged.
- Do not use the unavailable active-database `idempotency` capability; never silently retry an unresolved POST.
- Run every Rust command through `mise exec --`; run focused tests during each TDD cycle before broader verification.
- Keep all existing unrelated worktree changes untouched and commit only the files named by each task.

---

### Task 1: Add decision reads and the Web-only revision guard

**Files:**
- Modify: `src/app.rs`
- Modify: `src/store/sqlite.rs`
- Test: `src/store/sqlite.rs` unit tests
- Test: `tests/cli_decisions.rs` only when an existing call site needs the preserved `None` guard argument

**Interfaces:**
- `Database::list_decisions(project_name: &str, number: i64) -> Result<Vec<crate::domain::DecisionRequest>, crate::error::AppError>` returns all requests for one Issue ordered by creation time.
- `App::list_decisions(project: &str, number: i64) -> Result<Vec<crate::domain::DecisionRequest>, crate::error::AppError>` audits the read as `decision_list` and preserves the existing invalid-number and audit-failure behavior.
- `App::resolve_decision(request_id, answer, expected_revision, resolution, context)` and `Database::resolve_decision(request_id, answer, expected_revision, resolution, context)` accept `Option<i64>`; CLI callers pass `None`, and the Web caller passes the displayed revision.
- `Database::resolve_decision` compares `expected_revision` to the Issue revision inside its existing immediate transaction before any state or decision update.

- [x] **Step 1: Write the failing decision-read test**

Add a unit test beside the existing SQLite tests that initializes a temporary database, creates `bettr#1`, requests two decisions with two agent contexts, and calls `list_decisions("bettr", 1)`. Assert that both UUIDs and questions are returned in `created_at` order and that a different Issue number returns the existing `not_found` error.

```rust
let decisions = database.list_decisions("bettr", 1).unwrap();
assert_eq!(decisions.len(), 2);
assert_eq!(decisions[0].question, "Choose the parser");
assert_eq!(decisions[1].question, "Choose the rollout");
```

- [x] **Step 2: Run the read test and verify the expected red result**

Run: `mise exec -- cargo test store::sqlite::tests::list_decisions_returns_requests_in_creation_order`

Expected: compilation failure because `Database::list_decisions` does not exist. This is the intended missing-interface failure.

- [x] **Step 3: Implement the audited decision read**

Reuse the column order and `decision_from_row` parser already used by decision resolution. Add the project/Issue join and `ORDER BY request.created_at ASC`. Add the App wrapper with `decision_list` success/failure auditing, and include `decision_list` in the audit projection allowlists with target kind `issue`, project and revision fields, and no changed fields.

- [x] **Step 4: Run the read test and verify green**

Run: `mise exec -- cargo test store::sqlite::tests::list_decisions_returns_requests_in_creation_order`

Expected: PASS, with no new warnings.

- [x] **Step 5: Write the failing expected-revision test**

Add a SQLite unit test that requests one decision, records the resulting Issue revision, passes a different positive revision to `Database::resolve_decision`, and asserts `RevisionConflict { current_revision }`. Query SQLite afterward and assert the Issue remains `blocked`, the request remains `open`, and no `decision_resolved` or terminal transition event was appended.

```rust
let result = database.resolve_decision(
    request.id,
    "Use option A",
    Some(requested_revision - 1),
    crate::domain::DecisionResolutionInput::new(
        crate::domain::IssueState::Todo,
        None,
        None,
        None,
        None,
    ),
    &human_context,
);
assert!(matches!(
    result,
    Err(crate::error::AppError::RevisionConflict { .. })
));
```

- [x] **Step 6: Run the revision test and verify the expected red result**

Run: `mise exec -- cargo test store::sqlite::tests::decision_resolution_rejects_a_stale_expected_revision`

Expected: compilation failure because the existing resolution signature has no expected-revision parameter.

- [x] **Step 7: Implement the optional guard without changing the CLI contract**

Thread `Option<i64>` through `App::resolve_decision` and `Database::resolve_decision`. Validate a supplied revision as positive in the App layer. After loading the request's Issue inside the immediate transaction, return `RevisionConflict` when the expected value differs. Keep CLI `main.rs` calls passing `None`, preserve domain validation ordering, and leave all existing decision transition rules unchanged.

- [x] **Step 8: Run focused decision tests**

Run: `mise exec -- cargo test store::sqlite::tests::list_decisions_returns_requests_in_creation_order`, `mise exec -- cargo test store::sqlite::tests::decision_resolution_rejects_a_stale_expected_revision`, and `mise exec -- cargo test --test cli_decisions`

Expected: PASS. Existing CLI decision tests must retain their current behavior because the CLI passes no Web-only guard.

- [x] **Step 9: Commit the contract-preserving store change**

```sh
git add src/app.rs src/store/sqlite.rs tests/cli_decisions.rs
git commit -m "Add decision reads and revision guard"
```

### Task 2: Enrich Web read responses and add the resolve endpoint

**Files:**
- Modify: `src/web.rs`
- Modify: `tests/web_api.rs`

**Interfaces:**
- Web list/status items retain their existing flattened Issue fields and add `wait`, `unresolved_decision_count`, and `decision_questions` when applicable.
- `GET /api/issues/:number?project=<name>` adds `decisions`, `dependencies`, and `wait` while retaining `issue` and `history`.
- `POST /api/decisions/<uuid>/resolve` accepts `{expected_revision, answer, next_state, summary?, verification?, reason?, wait_kind?}` and returns the existing resolved `DecisionRequest` envelope.
- The request parser accepts bounded bodies for POST, rejects malformed or oversized bodies as `invalid_input`, and preserves the existing GET/static routes.

- [x] **Step 1: Extend the test fixture with an open decision and a blocked Issue**

In `tests/web_api.rs`, add helpers that use the existing test CLI to create an Issue, request one or two decisions as an agent, and optionally block an Issue with each existing `wait_kind`. Add `WebProcess::post_json` that sends `Content-Type: application/json`, `Content-Length`, and the encoded body, then parses the same status/body tuple as `get`.

- [x] **Step 2: Write the failing read-model and POST tests**

Add tests that:

1. assert `/api/status` and `/api/issues?project=bettr&include_done=true` expose the human wait label, reason, unresolved count, and question;
2. assert Issue detail exposes two decisions, dependencies, and a non-raw wait explanation;
3. POST a valid `todo` resolution and assert HTTP 200, the returned request is resolved by a human, the Issue is `todo` at the next revision, and a fresh detail response shows the updated decision/history;
4. POST with a stale `expected_revision` and assert HTTP 409/code `revision_conflict`, with the request still open and Issue state/revision unchanged;
5. POST `done` while another decision remains open and assert the existing conflict message and no partial write;
6. POST `in_progress`, unknown fields, missing state-specific fields, malformed JSON, invalid UUID, and non-human server context and assert the existing input/conflict codes;
7. hold an SQLite immediate transaction while posting and assert HTTP 503/code `database_busy` after releasing the lock;
8. assert the server still prints and binds a `127.0.0.1` URL, allows only the exact resolve POST route, and returns 405 for unrelated POST routes.

- [x] **Step 3: Run the focused Web tests and verify the expected red result**

Run: `mise exec -- cargo test --test web_api`

Expected: FAIL because the status/detail projection and POST route do not exist. Existing read-only tests should continue to identify the exact missing behavior.

- [x] **Step 4: Add the Web projection types and wait-context derivation**

Define private serializable Web types in `src/web.rs`. Convert the existing `Status` lists without altering `domain::Status`; enrich attention and blocked items by reading decisions, history, and dependencies through App. Derive wait context in this order: open decisions as `Human decision`, latest `issue_blocked` metadata using the existing `WaitKind`, dependency context, then the explicit missing-details message. Keep labels and reasons visible while hiding raw UUIDs from the list copy.

- [x] **Step 5: Extend detail and list routes**

Call `app.list_decisions` and `app.list_dependencies` for detail. Return the existing issue/history fields plus the new read model. Make `/api/issues` return the same enriched item shape used by the board and project list, with empty context for non-blocked/non-attention items.

- [x] **Step 6: Extend the bounded HTTP request parser**

Retain the existing request-line/header limits, store case-insensitive headers, parse one decimal `Content-Length`, cap the body at 64 KiB, and read exactly that body. Reject unsupported transfer encoding, malformed lengths, and body overflows with `invalid_input`. Do not add CORS or a second listener.

- [x] **Step 7: Implement the exact resolve route**

Parse the UUID from `/api/decisions/<uuid>/resolve`, deserialize a `deny_unknown_fields` request type, require a positive `expected_revision`, resolve the current process context, and call `App::resolve_decision` with `Some(expected_revision)`. Use the existing `status_code` mapping so invalid input, conflict/revision, not-found, busy, and internal errors preserve their current HTTP behavior.

- [x] **Step 8: Run the focused Web tests and verify green**

Run: `mise exec -- cargo test --test web_api`

Expected: PASS for read projection, successful resolution, atomic stale revision behavior, multiple decisions, permission/input/busy errors, and loopback/method boundaries.

- [x] **Step 9: Commit the Web API change**

```sh
git add src/web.rs tests/web_api.rs
git commit -m "Expose blocked context and decision resolution"
```

### Task 3: Render waiting context and accessible resolution forms

**Files:**
- Modify: `src/web/app.js`
- Modify: `src/web/app.css`
- Modify: `tests/frontend/detail.test.js`
- Modify: `tests/frontend/app-controller.test.js`
- Create: `tests/frontend/decisions.test.js`
- Modify: `tests/frontend/support.js` when shared response fixtures need the new Web fields

**Interfaces:**
- `renderDetail` consumes `response.data.wait`, `response.data.decisions`, and `response.data.dependencies`.
- `issueRow` and `kanbanCard` render `wait.label`, `wait.reason`, `unresolved_decision_count`, and `decision_questions` without requiring a raw enum or UUID.
- Each open request renders a `form[data-decision-form]` with `answer`, `next_state`, and state-specific controls.
- A POST uses the displayed `issue.revision`, keeps the form locked during submission, and calls `renderDetail` plus `pollStatus` after success.

- [x] **Step 1: Write failing frontend tests for list/detail context**

Add fixtures with a blocked Issue, a human wait object, two open decisions, requester names, and dependencies. Assert that board/list/detail DOM contains “Human decision”, the reason, the unresolved count, both questions, requester/created labels, and dependency text. Assert that a no-decision response renders the explicit decision empty state.

- [x] **Step 2: Run the focused frontend tests and verify the expected red result**

Run: `mise exec -- npm run test:frontend -- tests/frontend/detail.test.js tests/frontend/app-controller.test.js tests/frontend/decisions.test.js`

Expected: FAIL because the existing controller does not render Web wait data or decision forms.

- [x] **Step 3: Add semantic read-model rendering**

Add escaped wait-summary helpers, a waiting-context section, a decisions section, and a dependency section to `renderDetail`. Keep `article`, headings, `dl`, and existing activity semantics. Add compact wait/question summaries to rows and cards, and preserve focus when list rendering refreshes.

- [x] **Step 4: Write failing frontend tests for state-specific fields and submission safety**

In `decisions.test.js`, mount the existing shell and mock `fetch` for detail GET, resolve POST, and refreshed detail/status responses. Assert one form per open request, native `required` attributes for `todo`, `blocked`, `done`, and `cancelled`, omission of hidden optional fields, one POST despite two submit events, disabled button text while pending, and a successful GET after the POST.

```js
expect(form.elements.answer.required).toBe(true);
select.value = "done";
select.dispatchEvent(new Event("change", { bubbles: true }));
expect(form.elements.summary.required).toBe(true);
expect(form.elements.reason.required).toBe(false);
```

- [x] **Step 5: Run the submission tests and verify the expected red result**

Run: `mise exec -- npm run test:frontend -- tests/frontend/decisions.test.js`

Expected: FAIL because no resolve form or POST handler exists.

- [x] **Step 6: Implement dynamic validation and guarded submission**

Use semantic `fieldset`/`legend`, labels, and a `select` with only the four valid states. Toggle `hidden`, `disabled`, and `required` together. On submit, call native validity checks, set a per-form pending flag, disable the submit control, send the bounded JSON body to the exact route, and ignore subsequent submit events while pending. On success, reload detail and poll status. On conflict, revision, permission, input, busy, or unknown outcome, keep the answer values, announce a human-readable message, and expose an explicit reload action without automatic retry.

- [x] **Step 7: Add accessibility and state styling**

Add readable panel/card styles, text-based wait labels, error/success live-region styles, disabled control styles, and responsive/reduced-motion rules consistent with the existing CSS. Keep visible `:focus-visible` behavior and ensure colors are not the only wait/status signal.

- [x] **Step 8: Run the focused frontend tests and verify green**

Run: `mise exec -- npm run test:frontend -- tests/frontend/detail.test.js tests/frontend/app-controller.test.js tests/frontend/decisions.test.js`

Expected: PASS for rendering, multiple/empty decisions, all required-field combinations, keyboard-accessible labels/live regions, double-submit prevention, error/reload behavior, and post-success refresh.

- [x] **Step 9: Commit the frontend change**

```sh
git add src/web/app.js src/web/app.css tests/frontend/decisions.test.js
git commit -m "Add accessible decision resolution forms"
```

### Task 4: Document the local write boundary and operator flow

**Files:**
- Modify: `README.md`
- Modify: `docs/design.md`
- Modify: `docs/implementation-roadmap.md`
- Test: `tests/web_api.rs` static/documentation assertions only when a documented route or boundary is contract-tested

**Interfaces:**
- README and design/roadmap text describe `bettr web` as loopback-only with one human decision resolve exception to the read-only view.
- Documentation names the resolve route, displayed-revision requirement, human-only authority, state-specific fields, reload-on-conflict behavior, and the absence of external authentication/public exposure.
- Documentation explicitly excludes idempotency work for this feature and JSONL audit work for Issue #9.

- [x] **Step 1: Write the failing documentation assertions**

Add focused string assertions only if the existing documentation contract test structure supports them; otherwise inspect the exact conflicting sentences with `rg` and use the document diff as the red/green review artifact. The required old claims to remove are “CLI remains the only mutation path” and “read-only” when they describe the whole Web UI.

- [x] **Step 2: Update the operator-facing documentation**

Describe the command, loopback address, detail flow, resolve payload semantics, valid next states, required fields, errors, and no-auth/local-only boundary. Keep unrelated release, audit, and idempotency documentation unchanged.

- [x] **Step 3: Verify documentation consistency**

Run: `rg -n "CLI remains the only mutation|read-only|loopback|decision|resolve|JSONL" README.md docs/design.md docs/implementation-roadmap.md`

Expected: the Web sections consistently describe the single resolve write route, local boundary, and out-of-scope JSONL/idempotency behavior without contradicting the implementation spec.

- [x] **Step 4: Commit the documentation change**

```sh
git add README.md docs/design.md docs/implementation-roadmap.md
git commit -m "Document local decision resolution flow"
```

### Task 5: Run full verification and record the handoff

**Files:**
- Modify: Issue #19 conversation only for verification and remaining-scope comments

**Interfaces:**
- The final branch contains the approved design commit plus implementation/documentation commits.
- Issue #19 records the verified commit, exact commands, pass/fail outcomes, and the explicit no-merge/no-PR/no-worktree-removal boundary.

- [x] **Step 1: Check runtime resolution before Rust verification**

Run: `command -v rustc && rustc --version`, `command -v cargo && cargo --version`, and `command -v node && node --version`. Use `mise exec --` for all runtime commands that follow.

- [x] **Step 2: Run formatting and focused suites**

Run: `mise exec -- cargo fmt --check`, `mise exec -- cargo test`, `mise exec -- cargo test --test web_api`, `mise exec -- cargo test --test cli_decisions`, and `mise exec -- npm test`.

Expected: all commands exit 0 with no formatting errors, test failures, or unhandled warnings.

- [x] **Step 3: Run the required full Rust checks**

Run: `mise exec -- cargo test` and `mise exec -- cargo clippy --all-targets -- -D warnings`.

Expected: all Rust tests pass and clippy emits no denied warnings.

- [x] **Step 4: Build the release binary**

Run: `mise exec -- cargo build --release`.

Expected: the binary builds from the same committed source as the tests.

- [x] **Step 5: Inspect the final diff and worktree**

Run: `git diff --check`, `git status --short --branch`, and `git --no-pager log --oneline --decorate -8`. Confirm only the approved spec, plan, implementation, test, and documentation files are changed; do not rebase, merge, create a PR, or remove the worktree.

- [ ] **Step 6: Record verification in Issue #19**

Add one `[Conversation update]` comment containing the verified commit, test commands/results, loopback and human-only safety behavior, and any remaining limitation. Re-read the Issue and recent history, then transition to `done` only if the requested scope and verification evidence are complete; otherwise record the concrete blocker and keep the Issue active.

- [x] **Step 7: Commit any final verification-only correction**

If verification exposes a required correction, return to the smallest relevant task, add a failing regression test first, re-run its focused suite, and commit the correction with an English capitalized message. If no correction is needed, make no empty commit.
