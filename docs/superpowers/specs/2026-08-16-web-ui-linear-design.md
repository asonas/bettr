# bettr Web UI Design

> Status: approved design, implementation in progress

## Goal

Provide a local, read-oriented supervisor view for bettr that makes cross-project attention, issue state, and recent changes easy for a human to scan.

## Flow

`Overview -> project/issue list -> issue detail`

The sidebar contains Overview, Projects, Recent, and Search. Overview follows the existing `bettr status` order: attention required, stale work, blocked issues, recently completed or cancelled issues, and active work.

Issue detail is a URL-addressable two-column view. The main column contains the description and chronological comments/activity. The property rail contains state, priority, project, assignee, execution context, revision, and timestamps.

## Update indicator

The server stays local-only and the browser polls a read endpoint. When the observed snapshot changes, a sticky non-modal indicator says that new updates are available and reports the count when it can be derived. The list does not reorder until the user applies the refresh; route, filters, and selected issue are preserved. The indicator uses text and an icon, not color alone, and is announced politely to assistive technology.

## Scope

- Read-only web UI; CLI remains the mutation interface.
- Loopback binding only.
- List-first presentation; board view, subscriptions, and a Linear-style notification inbox are out of scope.
- Desktop and tablet are the initial targets; narrow layouts collapse the property rail.
- Empty, loading, database-lock, and stale-data states are explicit.

## Stack decision

The MVP uses the existing Rust binary and standard-library `TcpListener` with embedded HTML, CSS, and JavaScript assets. This keeps the first UI local, fast to start, and free of a new runtime or network dependency. A later frontend build system can replace the assets without changing the HTTP data contract.

## Accessibility and visual direction

Use a dense neutral workspace with a single bettr accent, visible keyboard focus, semantic headings and landmarks, text-plus-icon status indicators, reduced-motion-safe transitions, and readable timestamp/ID utility text. The visual language takes Linear's information hierarchy and keyboard-first flow as inspiration without copying its brand assets.
