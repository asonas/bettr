import { getAllByRole, getByRole } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

const status = {
  attention: [],
  stale: [],
  blocked: [],
  recently_completed: [],
  active: [{
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
  }],
};

function mountShell() {
  document.body.innerHTML = `
    <main id="main"><div id="app"></div></main>
    <span id="sync-label"></span><span id="breadcrumbs"></span>
    <div id="connection-state"></div><div id="project-nav-list"></div>
    <button id="updated-nav" type="button" aria-label="Updated Issues" aria-expanded="false"><span id="updated-count"></span></button>
    <div id="updated-menu" role="menu" aria-label="Updated Issues" hidden></div>
    <button id="search-nav"></button><button id="theme-toggle"></button>
  `;
}

describe("web controller", () => {
  it("renders five status columns and an Issue card from a status response", async () => {
    mountShell();
    const { createWebController } = await import("../../src/web/app.js");
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ data: status }) });
    const controller = createWebController({ fetch });

    await controller.pollStatus();

    expect(getAllByRole(document, "heading", { level: 2 }).map((heading) => heading.textContent)).toEqual([
      "Todo",
      "In progress",
      "Blocked",
      "Done",
      "Cancelled",
    ]);
    expect(getByRole(document, "button", { name: /Ship the board/ })).toBeTruthy();
  });
});
