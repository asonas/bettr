import { allIssues, applyStatusUpdate, kanbanColumns } from "./state.js";

const statusLabels = {
  todo: "Todo",
  in_progress: "In progress",
  blocked: "Blocked",
  done: "Done",
  cancelled: "Cancelled",
};

export function createWebController({
  document: documentRef = globalThis.document,
  window: windowRef = globalThis.window,
  fetch: fetchRef = globalThis.fetch,
  state: stateApi = { allIssues, applyStatusUpdate, kanbanColumns },
} = {}) {
  const document = documentRef;
  const window = windowRef;
  const fetch = fetchRef;
  const location = window.location;
  const app = document.querySelector("#app");
  const syncLabel = document.querySelector("#sync-label");
  const breadcrumbs = document.querySelector("#breadcrumbs");
  const updatedNav = document.querySelector("#updated-nav");
  const updatedCount = document.querySelector("#updated-count");
  const updatedMenu = document.querySelector("#updated-menu");
  const state = { status: null, snapshot: "", updatedIssues: new Set(), project: "" };
  let statusPollInFlight = false;
  let projectNavInFlight = false;
  let projectNavSnapshot = "";
  let pollTimer;

  function escapeHtml(value) {
    return String(value ?? "").replace(/[&<>'"]/g, (character) => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;",
    }[character]));
  }

  function formatDate(value) {
    if (!value) return "—";
    return new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(new Date(value));
  }

  function issueKey(issue) { return `${issue.project}#${issue.number}`; }

  function issueRow(item) {
    const issue = item.issue || item;
    const project = item.project || issue.project || "";
    const priority = issue.priority ? `<span class="priority ${escapeHtml(issue.priority)}">${escapeHtml(issue.priority)}</span>` : "";
    return `<button class="issue-row" type="button" data-project="${escapeHtml(project)}" data-number="${issue.number}">
      <span class="issue-state-dot ${escapeHtml(issue.state)}" aria-hidden="true"></span>
      <span class="issue-main"><span class="issue-title">${escapeHtml(issue.title)}</span><span class="issue-meta"><span class="issue-key">${escapeHtml(project)}#${issue.number}</span>${priority}<span>${escapeHtml(statusLabels[issue.state] || issue.state)}</span></span></span>
      <span class="issue-updated">${formatDate(issue.updated_at)}</span>
    </button>`;
  }

  function issueList(items, emptyTitle = "No matching Issues", emptyMessage = "Try a different filter to find another Issue.") {
    if (!items.length) return `<div class="empty-state"><strong>${emptyTitle}</strong><span>${emptyMessage}</span></div>`;
    return `<div class="issue-list">${items.map(issueRow).join("")}</div>`;
  }

  function kanbanCard(item) {
    const issue = item.issue || item;
    const project = item.project || issue.project || "";
    const key = issueKey({ project, number: issue.number });
    const priority = issue.priority ? `<span class="priority ${escapeHtml(issue.priority)}">${escapeHtml(issue.priority)}</span>` : "";
    const assignee = issue.assignee_name ? `<span>${escapeHtml(issue.assignee_name)}</span>` : "";
    const updated = state.updatedIssues.has(key);
    return `<button class="kanban-card${updated ? " is-updated" : ""}" type="button" data-project="${escapeHtml(project)}" data-number="${issue.number}"${updated ? ` data-updated="true"` : ""}>
      <span class="kanban-card-header"><span class="issue-key">${escapeHtml(key)}</span>${updated ? `<span class="sr-only">Updated</span>` : ""}</span>
      <span class="kanban-card-title">${escapeHtml(issue.title)}</span>
      <span class="kanban-card-meta">${priority}${assignee}</span>
      <time class="kanban-card-time" datetime="${escapeHtml(issue.updated_at)}">${formatDate(issue.updated_at)}</time>
    </button>`;
  }

  function renderProjects(project = "") {
    state.project = project;
    setActiveProjectNav(project);
    breadcrumbs.innerHTML = project ? `<a href="#/projects">Projects</a><span> / ${escapeHtml(project)}</span>` : "<span>Projects</span>";
    app.innerHTML = `<div id="kanban-board" class="kanban-board" aria-label="Issue board"></div>`;
    renderKanban();
  }

  function renderKanban() {
    const board = document.querySelector("#kanban-board");
    if (!board) return;
    if (!state.status) {
      board.innerHTML = `<div class="loading-state"><span class="loader" aria-hidden="true"></span><span>Loading Issues</span></div>`;
      return;
    }
    const focusedCard = document.activeElement?.matches(".kanban-card") ? issueKey({ project: document.activeElement.dataset.project, number: document.activeElement.dataset.number }) : "";
    const issues = stateApi.allIssues(state.status).filter((item) => {
      if (state.project && item.project !== state.project) return false;
      return true;
    });
    board.innerHTML = stateApi.kanbanColumns.map(([key, title]) => {
      const columnIssues = issues.filter((item) => item.issue.state === key);
      return `<section class="kanban-column" data-state="${key}"><header class="kanban-column-header"><h2>${title}</h2><span>${columnIssues.length}</span></header><div class="kanban-cards">${columnIssues.length ? columnIssues.map(kanbanCard).join("") : `<div class="kanban-empty">No Issues</div>`}</div></section>`;
    }).join("");
    bindIssueRows();
    if (focusedCard) [...document.querySelectorAll(".kanban-card")].find((card) => issueKey({ project: card.dataset.project, number: card.dataset.number }) === focusedCard)?.focus();
  }

  async function renderDetail(project, number) {
    state.updatedIssues.delete(issueKey({ project, number }));
    renderUpdatedMenu();
    breadcrumbs.innerHTML = `<a href="#/projects">Projects</a><span> / ${escapeHtml(project)}#${number}</span>`;
    app.innerHTML = `<div class="loading-state"><span class="loader" aria-hidden="true"></span><span>Loading Issue</span></div>`;
    const response = await api(`/api/issues/${encodeURIComponent(number)}?project=${encodeURIComponent(project)}`);
    const issue = response.data.issue;
    const history = response.data.history || [];
    const activity = history.map((event) => {
      const body = activityBody(event);
      const label = event.event_type === "comment_added" ? "Comment" : (event.event_type || "Activity").replaceAll("_", " ");
      const actor = event.context?.agent || event.context?.operator || event.context?.kind || "system";
      const session = event.context?.session_id ? ` · ${event.context.session_id}` : "";
      return `<article class="activity-item"><div class="activity-meta"><span class="activity-type">${escapeHtml(label)} · ${escapeHtml(actor)}${escapeHtml(session)}</span><time datetime="${escapeHtml(event.created_at)}">${formatDate(event.created_at)}</time></div><p class="activity-body">${escapeHtml(body || "Change recorded")}</p></article>`;
    }).join("");
    app.innerHTML = `<div class="detail-layout"><article><p class="eyebrow">${escapeHtml(project)} / Issue ${number}</p><p class="detail-key">${escapeHtml(issueKey({ project, number }))} · revision ${issue.revision}</p><h1 class="detail-title">${escapeHtml(issue.title)}</h1><div class="detail-body">${escapeHtml(issue.body || "")}</div><h2 class="activity-heading">Activity</h2><div class="activity-list">${activity || `<div class="empty-state"><strong>No activity yet</strong><span>Comments and state changes from the CLI will appear here.</span></div>`}</div></article><aside class="property-rail" aria-label="Issue properties"><dl class="property-list"><div><dt>State</dt><dd><span class="state-pill ${escapeHtml(issue.state)}">${escapeHtml(statusLabels[issue.state] || issue.state)}</span></dd></div><div><dt>Priority</dt><dd>${escapeHtml(issue.priority || "Not set")}</dd></div><div><dt>Assignee</dt><dd>${escapeHtml(issue.assignee_name || "Unassigned")}</dd></div><div><dt>Created</dt><dd>${formatDate(issue.created_at)}</dd></div><div><dt>Updated</dt><dd>${formatDate(issue.updated_at)}</dd></div><div><dt>Context</dt><dd>revision ${issue.revision}</dd></div></dl></aside></div>`;
  }

  async function renderRecent() {
    breadcrumbs.innerHTML = "<span>Recent</span>";
    const focusedIssue = document.activeElement?.matches(".issue-row") ? issueKey({ project: document.activeElement.dataset.project, number: document.activeElement.dataset.number }) : "";
    const items = stateApi.allIssues(state.status).sort((a, b) => new Date(b.issue.updated_at) - new Date(a.issue.updated_at));
    app.innerHTML = `<h1 class="sr-only">Recent</h1>${issueList(items, "No recent updates", "New updates will appear here.")}`;
    bindIssueRows();
    if (focusedIssue) [...document.querySelectorAll(".issue-row")].find((row) => issueKey({ project: row.dataset.project, number: row.dataset.number }) === focusedIssue)?.focus();
  }

  function activityBody(event) {
    const metadata = event.metadata || {};
    switch (event.event_type) {
      case "comment_added":
        return metadata.body || "Comment added";
      case "issue_created":
        return "Issue created";
      case "issue_updated": {
        const changedFields = Object.keys(metadata.changes || {});
        return changedFields.length ? `Changed: ${changedFields.join(", ")}` : "Issue updated";
      }
      default: {
        const from = statusLabels[metadata.from_state] || metadata.from_state;
        const to = statusLabels[metadata.to_state] || metadata.to_state;
        return from && to ? `${from} → ${to}` : "Change recorded";
      }
    }
  }

  async function route() {
    try {
      const parts = location.hash.replace(/^#\/?/, "").split("/").map(decodeURIComponent);
      const name = parts[0] || "projects";
      setActiveNav(name === "issues" ? "projects" : name);
      if (name === "projects") return renderProjects(parts[1] || "");
      if (name === "recent") return renderRecent();
      if (name === "issues" && parts[1] && parts[2]) return await renderDetail(parts[1], Number(parts[2]));
      return renderProjects();
    } catch (error) { renderError(error); }
  }

  function setActiveNav(name) {
    document.querySelectorAll("[data-nav]").forEach((link) => link.setAttribute("aria-current", link.dataset.nav === name ? "page" : "false"));
  }

  function setActiveProjectNav(project) {
    document.querySelectorAll("[data-project-nav]").forEach((link) => link.setAttribute("aria-current", link.dataset.projectNav === project ? "page" : "false"));
  }

  function updatedItems() {
    if (!state.status) return [];
    const issues = new Map(stateApi.allIssues(state.status).map((item) => [issueKey({ project: item.project, number: item.issue.number }), item]));
    return [...state.updatedIssues]
      .map((key) => issues.get(key))
      .filter(Boolean)
      .sort((a, b) => new Date(b.issue.updated_at) - new Date(a.issue.updated_at));
  }

  function closeUpdatedMenu() {
    if (!updatedMenu || !updatedNav) return;
    updatedMenu.hidden = true;
    updatedNav.setAttribute("aria-expanded", "false");
  }

  function handleUpdatedMenuClick(event) {
    const item = event.target?.closest?.("[data-updated-issue]");
    if (!item) return;
    const href = item.getAttribute("href");
    state.updatedIssues.delete(item.dataset.updatedIssue);
    closeUpdatedMenu();
    renderUpdatedMenu();
    if (href) location.hash = href;
  }

  function renderUpdatedMenu() {
    if (!updatedNav || !updatedCount || !updatedMenu) return;
    const items = updatedItems();
    const count = items.length;
    const label = `${count} update${count === 1 ? "" : "s"}`;
    updatedNav.hidden = count === 0;
    updatedCount.textContent = count ? label : "";
    updatedNav.setAttribute("aria-label", count ? `${label} — Updated Issues` : "Updated Issues");
    if (!count) {
      updatedMenu.innerHTML = "";
      closeUpdatedMenu();
      return;
    }
    const wasOpen = !updatedMenu.hidden;
    const focusedItem = document.activeElement?.matches("[data-updated-issue]") ? document.activeElement.dataset.updatedIssue : "";
    updatedMenu.innerHTML = `<div class="updated-menu-heading">Updated Issues</div>${items.map(({ project, issue }) => {
      const key = issueKey({ project, number: issue.number });
      return `<a class="updated-menu-item" role="menuitem" data-updated-issue="${escapeHtml(key)}" href="#/issues/${escapeHtml(encodeURIComponent(project))}/${issue.number}"><span class="updated-menu-title">${escapeHtml(issue.title)}</span><span class="updated-menu-meta">${escapeHtml(key)} · ${escapeHtml(statusLabels[issue.state] || issue.state)} · ${formatDate(issue.updated_at)}</span></a>`;
    }).join("")}`;
    updatedMenu.onclick = handleUpdatedMenuClick;
    updatedMenu.onkeydown = handleUpdatedMenuKeydown;
    updatedMenu.hidden = !wasOpen;
    updatedNav.setAttribute("aria-expanded", String(wasOpen));
    if (wasOpen && focusedItem) [...updatedMenu.querySelectorAll("[data-updated-issue]")].find((item) => item.dataset.updatedIssue === focusedItem)?.focus();
  }

  function toggleUpdatedMenu() {
    if (!updatedNav || !updatedMenu || !updatedItems().length) return;
    const open = updatedMenu.hidden;
    if (open) renderUpdatedMenu();
    updatedMenu.hidden = !open;
    updatedNav.setAttribute("aria-expanded", String(open));
    if (open) updatedMenu.querySelector("[data-updated-issue]")?.focus();
  }

  function handleDocumentKeydown(event) {
    if (event.key !== "Escape" || !updatedMenu || updatedMenu.hidden) return;
    closeUpdatedMenu();
    updatedNav?.focus();
  }

  function handleUpdatedMenuKeydown(event) {
    if (!updatedMenu || updatedMenu.hidden) return;
    const keys = ["ArrowDown", "ArrowUp", "Home", "End"];
    if (!keys.includes(event.key)) return;
    const items = [...updatedMenu.querySelectorAll("[data-updated-issue]")];
    if (!items.length) return;
    const current = items.indexOf(document.activeElement);
    const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : (current + (event.key === "ArrowUp" ? -1 : 1) + items.length) % items.length;
    event.preventDefault();
    items[next].focus();
  }

  function handleDocumentClick(event) {
    if (!updatedMenu || updatedMenu.hidden || event.target?.closest?.(".updated-control")) return;
    closeUpdatedMenu();
  }

  function bindIssueRows() {
    document.querySelectorAll(".issue-row, .kanban-card").forEach((row) => row.addEventListener("click", () => {
      const key = `${row.dataset.project}#${row.dataset.number}`;
      state.updatedIssues.delete(key);
      renderUpdatedMenu();
      location.hash = `#/issues/${encodeURIComponent(row.dataset.project)}/${row.dataset.number}`;
    }));
  }

  function renderError(error) {
    const message = error?.message || "Unable to load data";
    app.innerHTML = `<div class="error-state" role="alert"><strong>Unable to load this view</strong><span>${escapeHtml(message)}</span><button type="button" id="retry">Retry</button></div>`;
    document.querySelector("#retry").addEventListener("click", route);
  }

  async function api(path) {
    const response = await fetch(path, { headers: { Accept: "application/json" } });
    const body = await response.json();
    if (!response.ok || body.error) throw new Error(body.error?.message || "Unable to load data");
    return body;
  }

  async function loadProjectNavigation() {
    const list = document.querySelector("#project-nav-list");
    if (projectNavInFlight) return;
    projectNavInFlight = true;
    try {
      const projects = (await api("/api/projects")).data.filter((project) => !project.archived);
      const nextSnapshot = JSON.stringify(projects);
      if (nextSnapshot === projectNavSnapshot) return;
      const focusedProject = document.activeElement?.matches("[data-project-nav]") ? document.activeElement.dataset.projectNav : "";
      projectNavSnapshot = nextSnapshot;
      list.innerHTML = projects.length ? projects.map((project) => {
        const name = escapeHtml(project.name);
        return `<a class="project-nav-link" data-project-nav="${name}" aria-label="Project ${name}" title="${name}" href="#/projects/${escapeHtml(encodeURIComponent(project.name))}">${name}</a>`;
      }).join("") : `<span class="project-nav-state">No projects</span>`;
      setActiveProjectNav(state.project);
      if (focusedProject) [...document.querySelectorAll("[data-project-nav]")].find((link) => link.dataset.projectNav === focusedProject)?.focus();
    } catch (error) {
      projectNavSnapshot = "";
      if (!list.querySelector("[data-project-nav]")) list.innerHTML = `<span class="project-nav-state">Projects unavailable</span>`;
    } finally {
      projectNavInFlight = false;
    }
  }

  function snapshot(status) { return JSON.stringify(status); }

  function refreshCurrentView() {
    const name = location.hash.replace(/^#\/?/, "").split("/")[0] || "projects";
    if (name === "projects") renderKanban();
    if (name === "recent") renderRecent();
  }

  async function pollStatus() {
    if (statusPollInFlight) return;
    statusPollInFlight = true;
    try {
      const response = await api("/api/status");
      syncLabel.textContent = `Last checked ${formatDate(new Date().toISOString())}`;
      const firstPoll = !state.snapshot;
      const update = stateApi.applyStatusUpdate(firstPoll ? null : state.status, response.data, state.updatedIssues);
      const changed = update.changedKeys;
      state.status = response.data;
      state.snapshot = snapshot(response.data);
      state.updatedIssues = update.updatedIssues;
      renderUpdatedMenu();
      loadProjectNavigation();
      if (firstPoll) route();
      else if (changed.length) refreshCurrentView();
    } catch (error) {
      syncLabel.textContent = "Unable to refresh";
      document.querySelector("#connection-state").innerHTML = `<span class="connection-dot" style="background:var(--signal)" aria-hidden="true"></span><span>Reconnecting</span>`;
    } finally {
      statusPollInFlight = false;
    }
  }

  function start() {
    updatedNav?.addEventListener("click", toggleUpdatedMenu);
    document.addEventListener("keydown", handleDocumentKeydown);
    document.addEventListener("click", handleDocumentClick);
    document.querySelector("#theme-toggle").addEventListener("click", () => { const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark"; window.localStorage.setItem("bettr-theme", next); document.documentElement.dataset.theme = next; });
    window.addEventListener("hashchange", route);
    document.documentElement.dataset.theme = window.localStorage.getItem("bettr-theme") || (window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    loadProjectNavigation();
    pollStatus();
    pollTimer = window.setInterval(pollStatus, 4000);
    return controller;
  }

  function stop() {
    if (pollTimer) window.clearInterval(pollTimer);
    updatedNav?.removeEventListener("click", toggleUpdatedMenu);
    if (updatedMenu) {
      updatedMenu.onclick = null;
      updatedMenu.onkeydown = null;
    }
    document.removeEventListener("keydown", handleDocumentKeydown);
    document.removeEventListener("click", handleDocumentClick);
    window.removeEventListener("hashchange", route);
  }

  const controller = { state, start, stop, pollStatus, route, renderKanban, renderProjects, renderRecent, loadProjectNavigation, toggleUpdatedMenu, dismissUpdatedMenu: closeUpdatedMenu };
  return controller;
}

export function bootstrap(options) {
  return createWebController(options).start();
}

if (typeof document !== "undefined" && document.documentElement.dataset.bettrAutostart === "true") bootstrap();
