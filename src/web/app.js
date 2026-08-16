(() => {
  "use strict";

  const app = document.querySelector("#app");
  const banner = document.querySelector("#update-banner");
  const updateMessage = document.querySelector("#update-message");
  const applyUpdate = document.querySelector("#apply-update");
  const syncLabel = document.querySelector("#sync-label");
  const breadcrumbs = document.querySelector("#breadcrumbs");
  const state = { status: null, snapshot: "", pending: null, pendingCount: 0, issueRows: [], focusIndex: -1 };
  let goPending = false;
  let goTimer;

  const statusLabels = {
    todo: "Todo",
    in_progress: "In progress",
    blocked: "Blocked",
    done: "Done",
    cancelled: "Cancelled",
  };

  const sections = [
    ["attention", "Attention required", "判断待ち", "var(--danger)"],
    ["stale", "Stale work", "更新が止まっている進行中Issue", "var(--signal)"],
    ["blocked", "Blocked", "人間の判断または依存待ち", "var(--signal)"],
    ["recently_completed", "Recently completed", "最近完了または中止されたIssue", "var(--success)"],
    ["active", "Active", "現在稼働中のIssue", "var(--accent)"],
  ];

  function escapeHtml(value) {
    return String(value ?? "").replace(/[&<>'"]/g, (character) => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;",
    }[character]));
  }

  function formatDate(value) {
    if (!value) return "—";
    return new Intl.DateTimeFormat("ja-JP", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(value));
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

  function issueList(items, emptyTitle = "該当するIssueはありません") {
    if (!items.length) return `<div class="empty-state"><strong>${emptyTitle}</strong><span>フィルターを変更すると、別のIssueを確認できます。</span></div>`;
    return `<div class="issue-list">${items.map(issueRow).join("")}</div>`;
  }

  function setPage(title, eyebrow, summary = "") {
    breadcrumbs.innerHTML = `<span>${escapeHtml(title)}</span>`;
    return `<div class="page-header"><div><p class="eyebrow">${escapeHtml(eyebrow)}</p><h1>${escapeHtml(title)}</h1>${summary ? `<p class="page-summary">${escapeHtml(summary)}</p>` : ""}</div></div>`;
  }

  function renderOverview(status) {
    const nonEmpty = sections.filter(([key]) => status[key]?.length);
    const sectionsHtml = nonEmpty.map(([key, title, note, color]) => `<section class="status-section" style="--section-color:${color}"><div class="section-heading"><h2>${title}</h2><span>${status[key].length}件</span></div><p class="section-note">${note}</p>${issueList(status[key])}</section>`).join("");
    app.innerHTML = setPage("Overview", "Supervisor view", "いま介入すべき場所から、プロジェクト全体の動きを俯瞰します.") + (nonEmpty.length ? `<div class="section-grid">${sectionsHtml}</div>` : `<div class="all-clear"><span class="all-clear-mark" aria-hidden="true">✓</span><span>現在、注意が必要なIssueはありません。</span></div>`);
    bindIssueRows();
  }

  async function renderProjects() {
    breadcrumbs.innerHTML = "<span>Projects</span>";
    app.innerHTML = `<div class="page-header"><div><p class="eyebrow">Workspace</p><h1>Projects</h1><p class="page-summary">プロジェクトを横断してIssueの状態を確認します。</p></div></div><div class="toolbar"><label class="search-box"><span aria-hidden="true">⌕</span><span class="sr-only">Issueを検索</span><input id="issue-search" type="search" placeholder="タイトルや本文を検索" autocomplete="off" /></label><select id="project-filter" class="filter-select" aria-label="プロジェクトで絞り込む"><option value="">すべてのプロジェクト</option></select></div><div id="project-list"><div class="loading-state"><span class="loader" aria-hidden="true"></span><span>Issueを読み込んでいます</span></div></div>`;
    const projects = await api("/api/projects");
    const select = document.querySelector("#project-filter");
    select.insertAdjacentHTML("beforeend", projects.data.map((project) => `<option value="${escapeHtml(project.name)}">${escapeHtml(project.name)}</option>`).join(""));
    const load = async () => {
      const query = new URLSearchParams({ include_done: "true" });
      if (select.value) query.set("project", select.value);
      if (document.querySelector("#issue-search").value) query.set("query", document.querySelector("#issue-search").value);
      const issues = await api(`/api/issues?${query}`);
      document.querySelector("#project-list").innerHTML = issueList(issues.data);
      bindIssueRows();
    };
    select.addEventListener("change", load);
    document.querySelector("#issue-search").addEventListener("input", load);
    await load();
  }

  async function renderDetail(project, number) {
    breadcrumbs.innerHTML = `<a href="#/projects">Projects</a><span> / ${escapeHtml(project)}#${number}</span>`;
    app.innerHTML = `<div class="loading-state"><span class="loader" aria-hidden="true"></span><span>Issueを読み込んでいます</span></div>`;
    const response = await api(`/api/issues/${encodeURIComponent(number)}?project=${encodeURIComponent(project)}`);
    const issue = response.data.issue;
    const history = response.data.history || [];
    const activity = history.map((event) => {
      const body = event.event_type === "comment_added" ? event.metadata?.body : `${event.metadata?.from_state || ""} → ${event.metadata?.to_state || ""}`;
      const label = event.event_type === "comment_added" ? "Comment" : (event.event_type || "Activity").replaceAll("_", " ");
      const actor = event.context?.agent || event.context?.operator || event.context?.kind || "system";
      const session = event.context?.session_id ? ` · ${event.context.session_id}` : "";
      return `<article class="activity-item"><div class="activity-meta"><span class="activity-type">${escapeHtml(label)} · ${escapeHtml(actor)}${escapeHtml(session)}</span><time datetime="${escapeHtml(event.created_at)}">${formatDate(event.created_at)}</time></div><p class="activity-body">${escapeHtml(body || "変更が記録されました")}</p></article>`;
    }).join("");
    app.innerHTML = `<div class="detail-layout"><article><p class="eyebrow">${escapeHtml(project)} / Issue ${number}</p><p class="detail-key">${escapeHtml(issueKey({ project, number }))} · revision ${issue.revision}</p><h1 class="detail-title">${escapeHtml(issue.title)}</h1><div class="detail-body">${escapeHtml(issue.body || "")}</div><h2 class="activity-heading">Activity</h2><div class="activity-list">${activity || `<div class="empty-state"><strong>Activityはまだありません</strong><span>CLIでコメントや状態変更が行われると、ここに表示されます。</span></div>`}</div></article><aside class="property-rail" aria-label="Issue properties"><dl class="property-list"><div><dt>State</dt><dd><span class="state-pill ${escapeHtml(issue.state)}">${escapeHtml(statusLabels[issue.state] || issue.state)}</span></dd></div><div><dt>Priority</dt><dd>${escapeHtml(issue.priority || "未設定")}</dd></div><div><dt>Assignee</dt><dd>${escapeHtml(issue.assignee_name || "未割り当て")}</dd></div><div><dt>Created</dt><dd>${formatDate(issue.created_at)}</dd></div><div><dt>Updated</dt><dd>${formatDate(issue.updated_at)}</dd></div><div><dt>Context</dt><dd>revision ${issue.revision}</dd></div></dl></aside></div>`;
  }

  async function renderRecent() {
    breadcrumbs.innerHTML = "<span>Recent</span>";
    const items = [...(state.status?.recently_completed || []), ...(state.status?.active || [])].sort((a, b) => new Date(b.issue.updated_at) - new Date(a.issue.updated_at));
    app.innerHTML = setPage("Recent", "Activity", "最近変化したIssueを時系列で確認します。") + issueList(items, "最近の更新はありません");
    bindIssueRows();
  }

  async function route() {
    try {
      const parts = location.hash.replace(/^#\/?/, "").split("/").map(decodeURIComponent);
      const name = parts[0] || "overview";
      setActiveNav(name === "issues" ? "projects" : name);
      if (name === "projects") return await renderProjects();
      if (name === "recent") return renderRecent();
      if (name === "issues" && parts[1] && parts[2]) return await renderDetail(parts[1], Number(parts[2]));
      return renderOverview(state.status || { attention: [], stale: [], blocked: [], recently_completed: [], active: [] });
    } catch (error) { renderError(error); }
  }

  function setActiveNav(name) {
    document.querySelectorAll("[data-nav]").forEach((link) => link.setAttribute("aria-current", link.dataset.nav === name ? "page" : "false"));
  }

  function bindIssueRows() {
    state.issueRows = [...document.querySelectorAll(".issue-row")];
    state.issueRows.forEach((row, index) => { row.dataset.index = String(index); row.addEventListener("click", () => { location.hash = `#/issues/${encodeURIComponent(row.dataset.project)}/${row.dataset.number}`; }); });
  }

  function renderError(error) {
    const message = error?.message || "データを読み込めませんでした";
    app.innerHTML = `<div class="error-state" role="alert"><strong>表示できませんでした</strong><span>${escapeHtml(message)}</span><button type="button" id="retry">再試行</button></div>`;
    document.querySelector("#retry").addEventListener("click", route);
  }

  async function api(path) {
    const response = await fetch(path, { headers: { Accept: "application/json" } });
    const body = await response.json();
    if (!response.ok || body.error) throw new Error(body.error?.message || "データを読み込めませんでした");
    return body;
  }

  function snapshot(status) { return JSON.stringify(status); }

  function showUpdateBanner() {
    if (!state.pending) return;
    updateMessage.textContent = state.pendingCount ? `新しい更新があります（${state.pendingCount}件）` : "新しい更新があります";
    banner.hidden = false;
  }

  function changedIssueCount(previous, next) {
    const collect = (status) => Object.values(status || {}).flatMap((items) => Array.isArray(items) ? items : []).reduce((map, item) => {
      const issue = item.issue || item;
      map.set(`${item.project || ""}#${issue.number}`, `${issue.updated_at}:${issue.revision}:${issue.state}`);
      return map;
    }, new Map());
    const before = collect(previous);
    const after = collect(next);
    const keys = new Set([...before.keys(), ...after.keys()]);
    return [...keys].filter((key) => before.get(key) !== after.get(key)).length;
  }

  async function pollStatus() {
    try {
      const response = await api("/api/status");
      syncLabel.textContent = `最終確認 ${formatDate(new Date().toISOString())}`;
      if (!state.snapshot) { state.status = response.data; state.snapshot = snapshot(response.data); route(); return; }
      if (snapshot(response.data) !== state.snapshot) { state.pending = response.data; state.pendingCount = changedIssueCount(state.status, response.data); showUpdateBanner(); }
    } catch (error) {
      syncLabel.textContent = "更新を確認できません";
      document.querySelector("#connection-state").innerHTML = `<span class="connection-dot" style="background:var(--signal)" aria-hidden="true"></span><span>再接続待ち</span>`;
    }
  }

  applyUpdate.addEventListener("click", () => { if (!state.pending) return; state.status = state.pending; state.snapshot = snapshot(state.pending); state.pending = null; state.pendingCount = 0; banner.hidden = true; route(); });
  document.querySelector("#search-nav").addEventListener("click", () => { location.hash = "#/projects"; setTimeout(() => document.querySelector("#issue-search")?.focus(), 0); });
  document.querySelector("#theme-toggle").addEventListener("click", () => { const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark"; document.documentElement.dataset.theme = next; localStorage.setItem("bettr-theme", next); });
  document.addEventListener("keydown", (event) => {
    if (event.key === "/" && !["INPUT", "TEXTAREA", "SELECT"].includes(document.activeElement?.tagName)) { event.preventDefault(); location.hash = "#/projects"; setTimeout(() => document.querySelector("#issue-search")?.focus(), 0); }
    if (["INPUT", "TEXTAREA", "SELECT"].includes(document.activeElement?.tagName)) return;
    if (event.key.toLowerCase() === "g") { goPending = true; clearTimeout(goTimer); goTimer = setTimeout(() => { goPending = false; }, 900); return; }
    if (goPending && event.key.toLowerCase() === "o") { goPending = false; clearTimeout(goTimer); location.hash = "#/overview"; return; }
    const key = event.key.toLowerCase();
    if (!["j", "k"].includes(key) || !state.issueRows.length) return;
    event.preventDefault(); state.focusIndex = Math.max(0, Math.min(state.issueRows.length - 1, state.focusIndex + (key === "j" ? 1 : -1))); state.issueRows[state.focusIndex].focus();
  });
  window.addEventListener("hashchange", route);
  document.documentElement.dataset.theme = localStorage.getItem("bettr-theme") || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  pollStatus();
  setInterval(pollStatus, 4000);
})();
