# bettr Web UI Implementation Plan

> **For agentic workers:** This plan is executed inline in the approved web-ui worktree. Steps use checkbox syntax for tracking.

**Goal:** Add a loopback-only, read-only Linear-inspired supervisor UI with Overview, issue lists, issue detail, and a non-disruptive update indicator.

**Architecture:** Extend the existing Rust binary with a `web` subcommand that binds only to `127.0.0.1`. A small standard-library HTTP server exposes JSON read endpoints backed by the existing `App`/SQLite snapshot APIs and serves embedded HTML/CSS/JavaScript. The browser owns navigation state and polls a status snapshot without mutating the database.

**Tech Stack:** Rust 2024, `std::net::TcpListener`, existing `App`/`Database`/Serde types, embedded HTML/CSS/vanilla JavaScript, Rust unit/integration tests.

**Spec:** `docs/superpowers/specs/2026-08-16-web-ui-linear-design.md`

## Global Constraints

- Bind the server to loopback only; do not add network sharing or authentication to the MVP.
- Keep the web UI read-only; existing CLI commands remain the only mutation path.
- Preserve the existing SQLite identity preflight and read snapshot/concurrency behavior.
- Do not add a frontend runtime or dependency unless a test demonstrates the standard-library server is insufficient.
- Do not reorder visible issue rows while the user is reading; refresh only after explicit user action.
- Use accessible labels, focus states, text-plus-icon status meaning, and `prefers-reduced-motion`.

### Task 1: Read models and web command boundary

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/store/sqlite.rs`
- Modify: `src/domain.rs`
- Test: `tests/cli_help.rs`, `tests/web_api.rs`

**Interfaces:**
- `Command::Web(WebCommand)` parses `bettr web [--port PORT]`.
- Issue comments are represented by the existing `DomainEvent` history payload and do not require a new mutation or storage API.
- `web::run(database_path, port)` owns the loopback listener and dispatches read requests.

- [ ] Write a failing CLI help test asserting `web` and `--port` are documented.
- [ ] Run `cargo test --test cli_help web` and observe the missing command.
- [ ] Add the command type and main dispatch without starting a listener in help/version paths.
- [ ] Add the failing comment lookup/API tests described in Task 2.
- [ ] Implement the audited comment read boundary and endpoint response types.
- [ ] Run the focused tests and verify they pass.
- [ ] Commit the command boundary and read-model changes.

### Task 2: Loopback HTTP server and JSON endpoints

**Files:**
- Create: `src/web.rs`
- Create: `tests/web_api.rs`
- Modify: `src/main.rs`

**Interfaces:**
- `GET /api/status` returns the versioned JSON status object.
- `GET /api/projects` returns projects.
- `GET /api/issues?project=&query=&include_done=` returns issue list items.
- `GET /api/issues/:number?project=` returns `{ issue, comments, history }`.
- `GET /`, `/app.css`, and `/app.js` return embedded assets.
- Unknown routes return a JSON 404 without panicking.

- [ ] Write tests that start a listener on an ephemeral loopback port, create a fixture database, and assert status, list, detail, static, and 404 responses.
- [ ] Run the focused tests and observe failures for the missing module/routes.
- [ ] Implement a bounded HTTP request parser, path/query decoder, JSON response helper, and one-request-per-connection handler.
- [ ] Open the database through the existing identity-checked `Database::open` for every HTTP request; never bypass preflight.
- [ ] Implement the four API routes and static asset routes.
- [ ] Run the focused tests, then add a concurrency read test for two simultaneous status requests.
- [ ] Commit the server and endpoint implementation.

### Task 3: Overview, list, detail, and update indicator UI

**Files:**
- Create: `src/web/index.html`
- Create: `src/web/app.css`
- Create: `src/web/app.js`
- Modify: `src/web.rs`
- Test: `tests/web_api.rs`

**Interfaces:**
- Embedded assets are returned with correct content types and cache-safe headers.
- Browser routes are `#/overview`, `#/projects`, and `#/issues/:project/:number`.
- `app.js` polls `/api/status` and displays an explicit update banner without replacing the current view until the user applies it.

- [ ] Add static-content assertions for landmarks, skip link, update banner, `aria-live`, focus styles, and reduced-motion CSS.
- [ ] Run the assertions and observe failures before assets exist.
- [ ] Build the semantic shell: sidebar, main landmark, page heading, filter/search controls, and detail property rail.
- [ ] Render the status sections and list/detail states from JSON.
- [ ] Add update snapshot comparison, banner apply action, loading/empty/error/stale states, and keyboard navigation.
- [ ] Run asset assertions and manually exercise the flow with a local fixture.
- [ ] Commit the UI assets.

### Task 4: Documentation and verification

**Files:**
- Modify: `README.md`
- Modify: `docs/design.md`
- Modify: `docs/implementation-roadmap.md`
- Test: `tests/web_api.rs`, existing test suite

- [ ] Document `bettr web --port 0` and loopback behavior, routes, read-only scope, and update indicator semantics.
- [ ] Add a short operator flow for Overview -> list -> detail -> apply update.
- [ ] Run `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo build --release`.
- [ ] Run the web API test suite and inspect rendered HTML/CSS/JS through a local browser-capable check if available; record any unavailable browser checks explicitly.
- [ ] Review the diff against the design spec and commit only the intended files.
