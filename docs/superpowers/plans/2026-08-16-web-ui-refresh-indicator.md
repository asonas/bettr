# Web UI Refresh Indicator Implementation Plan

> **For agentic workers:** Execute this plan inline in the approved `web-ui-refresh-indicator` worktree. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the blocking update banner with a header update menu, make updated Issue cards identifiable by color, and use the full viewport for a calm five-column Kanban board.

**Architecture:** Keep the existing Rust-embedded HTML/CSS/Vanilla JavaScript architecture. Extend the injected `createWebController` state with a session-scoped update menu, render project navigation as text-only links, and keep status polling as the source of changed Issue keys. Use Vitest/jsdom for interaction tests and Rust tests only for the embedded asset/API boundary.

**Tech Stack:** Embedded HTML, CSS, Vanilla JavaScript ESM, Vitest, jsdom, `@testing-library/dom`, Rust HTTP server.

**Spec:** Approved conversation design on 2026-08-16: no update banner; header click opens a newest-first updated Issue menu; updated cards use `#25bbd4` as a small border/tint cue with accessible hidden text; five Kanban columns use available desktop width; redundant project heading copy and project dots are removed; Web UI copy is English.

## Global Constraints

- Do not add React, another frontend runtime, or a network dependency.
- Keep `bettr web` read-only, loopback-only, and Rust-embedded.
- Keep `#25bbd4` out of normal text on light surfaces; use it for the update border/tint and verify contrast.
- Do not use color as the only semantic cue; updated cards retain an accessible hidden label.
- Keep the update set in memory for the current page session; opening the Issue clears its key.
- Preserve keyboard support: header menu toggles with Enter/Space, closes with Escape or outside click, and menu items are native links/buttons with visible focus.

### Task 1: Specify the new controller behavior with failing DOM tests

**Files:**
- Modify: `tests/frontend/support.js`
- Modify: `tests/frontend/kanban.test.js`
- Modify: `tests/frontend/navigation.test.js`
- Create: `tests/frontend/update-menu.test.js`
- Modify: `tests/frontend/project-navigation.test.js`

**Interfaces:**
- The test shell exposes `#updated-nav`, `#updated-count`, `#updated-menu`, `#app`, `#breadcrumbs`, `#sync-label`, and `#project-nav-list`.
- The controller exposes `toggleUpdatedMenu()` and `dismissUpdatedMenu()` only through the returned test handle when needed by tests.

- [x] Add a failing test that an updated Issue adds a header count, opens a newest-first menu, and navigates to the selected Issue.
- [x] Add a failing test that an updated card has `is-updated`, a `data-updated="true"` hook, and an accessible hidden `Updated` label but no visible `Updated` text.
- [x] Add a failing test that opening an Issue removes its key from the update menu and count.
- [x] Add a failing test that Escape and outside clicks close the update menu.
- [x] Update the Kanban test to assert the update banner is absent and the updated card remains visible after polling.
- [x] Update project navigation tests to expect text-only links with no `.project-nav-dot` elements.
- [x] Run the focused tests and confirm they fail because the old banner and dot-based navigation remain.

### Task 2: Implement the minimal controller and markup changes

**Files:**
- Modify: `src/web/index.html`
- Modify: `src/web/app.js`
- Modify: `tests/frontend/support.js`

**Interfaces:**
- `state.updatedIssues` remains a `Set` of `project#number` keys.
- `renderUpdatedMenu()` derives unique menu entries from `state.status` and `state.updatedIssues`, sorted by `updated_at` descending.
- `renderProjects(project)` renders only the search toolbar and Kanban board; it does not create the old `Workspace`, title, or summary block.

- [x] Remove the update banner and its action elements from the HTML shell.
- [x] Add an accessible header update control and a hidden popover menu with stable IDs.
- [x] Remove `showUpdateBanner()` and update-poll calls to it.
- [x] Render updated cards with a non-visible accessible label and no visible `Updated` word.
- [x] Render the update menu on every changed-status redraw and after opening an Issue.
- [x] Add Escape and document-level outside-click handlers that only close the menu.
- [x] Clear the selected Issue key before routing to its detail view.
- [x] Remove project mark generation and render plain project links; preserve active state and focus restoration.
- [x] Run focused Vitest tests and confirm they pass.

### Task 3: Rework layout, copy, and color tokens

**Files:**
- Modify: `src/web/app.css`
- Modify: `src/web/index.html`
- Modify: `README.md`
- Modify: `docs/design.md`
- Modify: `docs/implementation-roadmap.md`

**Interfaces:**
- `--accent` becomes `#25bbd4` for the shared accent token.
- `--updated-surface` is a low-opacity mix of `#25bbd4` and the current surface; normal text remains `--ink`/`--ink-muted`.
- `.kanban-board` uses five equal flexible columns at desktop widths and horizontal scrolling only when the columns no longer fit.

- [x] Make the workspace and main content use the available viewport instead of a fixed `1160px` cap.
- [x] Ensure all five columns, including Cancelled, are visible at the supported desktop width without clipping.
- [x] Remove redundant page-header styles or leave only styles still used by detail/recent pages.
- [x] Add menu, updated-card, and focus styles consistent with the existing surface/radius system.
- [x] Remove decorative project dots and narrow-sidebar mark rules.
- [x] Set visible Web UI copy to English where touched, including loading, sync, project navigation, and empty states.
- [x] Verify `#25bbd4` is not used as light-surface text and the update cue is not color-only.

### Task 4: Boundary tests, documentation, and verification

**Files:**
- Modify: `tests/web_api.rs`
- Modify: `docs/superpowers/plans/2026-08-16-web-ui-refresh-indicator.md`

- [x] Update embedded asset assertions for the removed banner and new update-menu IDs.
- [x] Run `npm test` and record the frontend test count (8 files, 12 Vitest tests, 2 Node tests).
- [x] Run `cargo fmt --check`.
- [x] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [x] Run `cargo test --test web_api`.
- [x] Run `cargo test --tests`.
- [x] Run `cargo build --release`.
- [x] Run `git diff --check` and inspect the rendered source for accidental Japanese/old banner copy.
- [x] Commit only the controller, markup, CSS, tests, docs, and plan changes (`ba5139f`).
