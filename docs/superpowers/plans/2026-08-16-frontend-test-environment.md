# Frontend Test Environment Implementation Plan

> **For agentic workers:** Execute this plan inline in the approved `frontend-test-env` worktree. Steps use checkbox syntax for tracking.

**Goal:** Add a Vitest/jsdom test environment for bettr's Vanilla JS UI and move browser behavior checks out of Rust source-string assertions.

**Architecture:** Keep Rust responsible for loopback HTTP and embedded asset/API contracts. Refactor the browser script into a testable controller bootstrap with injected DOM/fetch dependencies, then exercise it through jsdom and Testing Library DOM queries while preserving the production script entry point.

**Tech Stack:** Node.js 24 LTS, npm, Vitest, jsdom, `@testing-library/dom`, embedded HTML/CSS/Vanilla JavaScript, Rust integration tests.

**Spec:** `docs/superpowers/specs/2026-08-16-frontend-test-environment-design.md`

## Global Constraints

- Do not add React, Preact, or another frontend runtime.
- Keep `bettr web` production delivery as Rust `include_str!` embedded source assets.
- Use accessible DOM queries for user-visible assertions where possible.
- Keep Rust Web API tests for HTTP and JSON contracts, not browser behavior.
- Do not add Playwright or a browser download in this phase.
- Follow red-green verification for each behavior test before implementation.

### Task 1: Frontend package and test runner

**Files:**
- Create: `package.json`
- Create: `package-lock.json`
- Create: `vitest.config.js`
- Create: `tests/frontend/setup.js`
- Test: `tests/frontend/smoke.test.js`

**Interfaces:**
- `npm test` runs the Vitest suite and the Node state projection tests.
- `npm run test:frontend:watch` runs Vitest watch mode.
- Vitest uses jsdom and loads `tests/frontend/setup.js` before tests.

- [x] Write a smoke test importing the existing state module and asserting the five Kanban columns.
- [x] Run `npm test -- --run tests/frontend/smoke.test.js` and observe the missing package/test environment failure.
- [x] Add the package scripts, Vitest jsdom config, and setup file.
- [x] Run the smoke test and verify it passes.
- [x] Commit the frontend test runner scaffold.

### Task 2: Testable browser controller boundary

**Files:**
- Modify: `src/web/state.js`
- Modify: `src/web/app.js`
- Modify: `src/web/index.html`
- Test: `tests/frontend/app-controller.test.js`

**Interfaces:**
- `src/web/state.js` exports the pure status projection functions while preserving `globalThis.BettrWebState` for the embedded browser entry.
- `src/web/app.js` exports `createWebController({ document, window, fetch, state })` for tests and calls `bootstrap()` only in the production browser entry.
- The controller exposes `pollStatus()`, `route()`, and `renderKanban()` only through the returned test handle; production behavior remains unchanged.

- [x] Write a failing controller test that mounts the HTML shell, feeds a status response, and expects five named columns and an Issue card.
- [x] Run the focused test and verify it fails because `createWebController` is not exported.
- [x] Extract DOM references, rendering, routing, and polling dependencies into the controller factory without changing output behavior.
- [x] Keep the production entry compatible with Rust's embedded `/state.js` and `/app.js` routes.
- [x] Run the focused test and verify it passes.
- [x] Commit the controller boundary.

### Task 3: Move browser behavior assertions to jsdom

**Files:**
- Create: `tests/frontend/kanban.test.js`
- Create: `tests/frontend/project-navigation.test.js`
- Create: `tests/frontend/polling.test.js`
- Create: `tests/frontend/detail.test.js`
- Create: `tests/frontend/navigation.test.js`
- Create: `tests/frontend/support.js`
- Modify: `src/web/app.css`
- Modify: `tests/web_api.rs`

**Interfaces:**
- Browser tests use `@testing-library/dom` queries and injected fetch responses.
- Rust tests retain asset route/content-type, API schema, non-GET, unknown-route, and concurrent-read assertions.

- [x] Add failing tests for status-column order, updated-card indicator, automatic movement, navigation retry, and focus restoration.
- [x] Run the focused frontend tests and verify each fails for the expected missing behavior.
- [x] Implement only the controller changes needed to make the behavior tests pass.
- [x] Remove Rust source-string assertions that duplicate browser behavior; retain API/asset boundary assertions.
- [x] Run `npm test` and the focused Rust Web API suite.
- [x] Commit the browser behavior tests and Rust test boundary cleanup.

### Task 4: Documentation and verification

**Files:**
- Modify: `README.md`
- Modify: `docs/design.md`
- Modify: `docs/implementation-roadmap.md`

- [x] Document `npm install`, `npm test`, the jsdom test boundary, and the retained Rust Web API checks.
- [x] Run `npm test` and record the test count.
- [x] Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --tests`, and `cargo build --release`.
- [x] Run `git diff --check` and inspect the final diff against the spec.
- [x] Commit only the intended frontend environment, test, boundary, and documentation files.
