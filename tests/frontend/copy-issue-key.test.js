import { getByRole } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { issue, jsonResponse, mountShell, statusWith } from "./support.js";

describe("Issue key copy actions", () => {
  it("copies the Issue key from a Kanban card without opening the Issue", async () => {
    mountShell();
    const clipboard = { writeText: vi.fn().mockResolvedValue(undefined) };
    const fetch = vi.fn((path) => path === "/api/status"
      ? Promise.resolve(jsonResponse(statusWith({ active: [issue()] })))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch, clipboard });

    window.location.hash = "";
    await controller.pollStatus();
    getByRole(document, "button", { name: "Copy bettr#1" }).click();

    await vi.waitFor(() => expect(clipboard.writeText).toHaveBeenCalledWith("bettr#1"));
    expect(window.location.hash).toBe("");
    expect(getByRole(document, "button", { name: "Copied bettr#1" })).toBeTruthy();
  });

  it("copies the Issue key from the Issue detail view", async () => {
    mountShell();
    const clipboard = { writeText: vi.fn().mockResolvedValue(undefined) };
    const fetch = vi.fn((path) => path.startsWith("/api/issues/")
      ? Promise.resolve(jsonResponse({ issue: issue(), history: [] }))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch, clipboard });

    window.location.hash = "#/issues/bettr/1";
    await controller.route();
    getByRole(document, "button", { name: "Copy bettr#1" }).click();

    await vi.waitFor(() => expect(clipboard.writeText).toHaveBeenCalledWith("bettr#1"));
  });

  it("announces when copying the Issue key fails", async () => {
    mountShell();
    const clipboard = { writeText: vi.fn().mockRejectedValue(new Error("Not allowed")) };
    const fetch = vi.fn((path) => path === "/api/status"
      ? Promise.resolve(jsonResponse(statusWith({ active: [issue()] })))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch, clipboard });

    window.location.hash = "#/projects";
    await controller.pollStatus();
    getByRole(document, "button", { name: "Copy bettr#1" }).click();

    await vi.waitFor(() => expect(getByRole(document, "status").textContent).toContain("Unable to copy bettr#1"));
  });
});
