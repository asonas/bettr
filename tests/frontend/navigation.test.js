import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { issue, jsonResponse, mountShell, statusWith } from "./support.js";

describe("navigation controls", () => {
  it("does not install the removed keyboard shortcut handler", () => {
    mountShell();
    const fetch = vi.fn((path) => path === "/api/status" ? new Promise(() => {}) : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });
    controller.start();
    const before = window.location.hash;

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "g" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "o" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "/" }));

    expect(window.location.hash).toBe(before);
    controller.stop();
  });

  it("closes the updated Issue menu with Escape and an outside click", async () => {
    mountShell();
    const fetch = vi.fn((path) => path === "/api/status" ? new Promise(() => {}) : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });
    controller.start();
    controller.state.status = statusWith({ active: [issue()] });
    controller.state.updatedIssues.add("bettr#1");
    controller.toggleUpdatedMenu();
    expect(document.querySelector("#updated-menu").hidden).toBe(false);

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(document.querySelector("#updated-menu").hidden).toBe(true);

    controller.toggleUpdatedMenu();
    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(document.querySelector("#updated-menu").hidden).toBe(true);
    controller.stop();
  });
});
