import { getByRole, queryByRole, within } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { issue, jsonResponse, mountShell, statusWith } from "./support.js";

describe("Kanban board", () => {
  it("moves an updated Issue to its new status column and marks it as updated", async () => {
    mountShell();
    const statusResponses = [
      jsonResponse(statusWith({ active: [issue()] })),
      jsonResponse(statusWith({ recently_completed: [issue({ state: "done", revision: 2, updated_at: "2026-08-16T08:01:00Z" })] })),
    ];
    const fetch = vi.fn((path) => path === "/api/status" ? Promise.resolve(statusResponses.shift()) : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    await controller.pollStatus();
    await controller.pollStatus();

    const inProgress = getByRole(document, "heading", { name: "In progress" }).closest(".kanban-column");
    const done = getByRole(document, "heading", { name: "Done" }).closest(".kanban-column");
    expect(queryByRole(inProgress, "button", { name: /Ship the board/ })).toBeNull();
    expect(within(done).getByRole("button", { name: /Ship the board/ })).toBeTruthy();
    const updatedCard = within(done).getByRole("button", { name: /Ship the board/ });
    expect(updatedCard.classList.contains("is-updated")).toBe(true);
    expect(updatedCard.dataset.updated).toBe("true");
    expect(updatedCard.querySelector(".sr-only").textContent).toBe("Updated");
    expect(updatedCard.querySelector(".update-indicator")).toBeNull();
    expect(document.querySelector("#update-banner")).toBeNull();
  });
});
