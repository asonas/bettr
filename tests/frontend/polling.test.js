import { getByRole } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { issue, jsonResponse, mountShell, statusWith } from "./support.js";

describe("polling redraws", () => {
  it("does not start a second status request while one is in flight", async () => {
    mountShell();
    let resolveStatus;
    const pendingStatus = new Promise((resolve) => { resolveStatus = resolve; });
    const fetch = vi.fn((path) => path === "/api/status" ? pendingStatus : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    const firstPoll = controller.pollStatus();
    const secondPoll = controller.pollStatus();
    expect(fetch).toHaveBeenCalledTimes(1);

    resolveStatus(jsonResponse(statusWith({ active: [issue()] })));
    await firstPoll;
    await secondPoll;
  });

  it("restores focus to a Kanban card after a polling update", async () => {
    mountShell();
    const statusResponses = [
      jsonResponse(statusWith({ active: [issue()] })),
      jsonResponse(statusWith({ active: [issue({ title: "Updated board", revision: 2, updated_at: "2026-08-16T08:01:00Z" })] })),
    ];
    const fetch = vi.fn((path) => path === "/api/status" ? Promise.resolve(statusResponses.shift()) : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    await controller.pollStatus();
    getByRole(document, "button", { name: /Ship the board/ }).focus();
    await controller.pollStatus();

    expect(document.activeElement).toBe(getByRole(document, "button", { name: /Updated board/ }));
  });

  it("restores focus to a Recent row after a polling redraw", async () => {
    mountShell();
    const fetch = vi.fn((path) => path === "/api/status" ? Promise.resolve(jsonResponse(statusWith({ active: [issue()] }))) : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    await controller.pollStatus();
    window.location.hash = "#/recent";
    await controller.route();
    getByRole(document, "button", { name: /Ship the board/ }).focus();
    await controller.renderRecent();

    expect(document.activeElement).toBe(getByRole(document, "button", { name: /Ship the board/ }));
  });
});
