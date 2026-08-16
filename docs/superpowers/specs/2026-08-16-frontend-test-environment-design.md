# bettr Frontend Test Environment Design

> Status: approved direction; implementation in progress

## Goal

Test the bettr Web UI as browser behavior instead of inspecting JavaScript source strings from Rust tests.

## Scope

- Keep the production UI as embedded HTML, CSS, and Vanilla JavaScript served by the Rust loopback server.
- Add a Node-based frontend test environment using Vitest and jsdom.
- Use Testing Library DOM queries for user-visible behavior; do not introduce React, Preact, or another component runtime.
- Test Kanban rendering, project navigation, polling updates, automatic status movement, update indicators, and focus restoration in JavaScript.
- Keep Rust Web API tests for HTTP routing, content types, embedded asset delivery, and JSON response contracts.
- Keep the existing Node state-model tests and expand them only where the browser behavior needs a pure-data contract.

## Architecture

`src/web/app.js` will expose a testable browser bootstrap/controller boundary while retaining a small production bootstrap at the bottom of the file. Tests will create a jsdom document, provide deterministic API responses through an injected fetch implementation, and assert DOM output and focus using accessible queries. `src/web/state.js` remains the pure status projection module and is imported by the browser controller and tests.

The Rust server continues to embed the source files with `include_str!`; no generated frontend bundle is required for production and no frontend runtime is shipped to users.

## Tooling

- Node.js 24 LTS already available through mise
- Vitest as the test runner
- jsdom as the browser-like DOM environment
- `@testing-library/dom` for semantic DOM assertions
- npm lockfile committed for reproducible development installs

## Test boundary

Frontend tests own behavior assertions such as:

- five status columns render in the configured order;
- an updated issue gets an `Updated` indicator and moves columns on the next poll;
- a project navigation request recovers after a failed request;
- focused cards and Recent rows retain focus across polling redraws;
- project navigation exposes identifiable marks in narrow layouts.

Rust tests own behavior assertions such as:

- static assets and `/state.js` are served from the expected routes;
- web API responses retain their schema and status codes;
- non-GET and unknown requests remain rejected;
- concurrent read requests remain safe.

## Non-goals

- No React migration.
- No frontend production bundling or change to the `bettr web` runtime command.
- No Playwright browser download in this phase; real-browser smoke tests can be added after the DOM suite is stable.
