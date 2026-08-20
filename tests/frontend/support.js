export function mountShell() {
  document.body.innerHTML = `
    <main id="main"><div id="app"></div></main>
    <span id="sync-label"></span><span id="breadcrumbs"></span>
    <div id="connection-state"></div><div id="project-nav-list"></div>
    <div id="copy-feedback" role="status" aria-atomic="true"></div>
    <button id="updated-nav" type="button" aria-label="Updated Issues" aria-expanded="false"><span id="updated-count"></span></button>
    <div id="updated-menu" role="menu" aria-label="Updated Issues" hidden></div>
    <button id="theme-toggle"></button>
  `;
}

export function issue(overrides = {}) {
  return {
    project: "bettr",
    number: 1,
    title: "Ship the board",
    body: "",
    state: "in_progress",
    priority: "high",
    assignee_name: "codex",
    revision: 1,
    created_at: "2026-08-16T08:00:00Z",
    updated_at: "2026-08-16T08:00:00Z",
    ...overrides,
  };
}

export function statusWith({ active = [], blocked = [], recently_completed = [] } = {}) {
  return { attention: [], stale: [], blocked, recently_completed, active };
}

export function jsonResponse(data) {
  return { ok: true, json: async () => ({ data }) };
}
