import { getByRole, getByText, queryByText, within } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { issue, jsonResponse, mountShell, statusWith } from "./support.js";

describe("updated Issue menu", () => {
  it("lists unique updated Issues newest first and opens the selected Issue", async () => {
    mountShell();
    const statusResponses = [
      jsonResponse(statusWith({ active: [issue()] })),
      jsonResponse(statusWith({ active: [
        issue({ number: 1, title: "Older board", revision: 2, updated_at: "2026-08-16T08:01:00Z" }),
        issue({ number: 2, title: "Newest board", revision: 1, updated_at: "2026-08-16T08:02:00Z" }),
      ] })),
    ];
    const fetch = vi.fn((path) => path === "/api/status" ? Promise.resolve(statusResponses.shift()) : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    await controller.pollStatus();
    await controller.pollStatus();
    controller.toggleUpdatedMenu();

    const menu = getByRole(document, "menu", { name: "Updated Issues" });
    expect(within(menu).getAllByRole("menuitem").map((item) => item.textContent)).toEqual([
      expect.stringContaining("Newest board"),
      expect.stringContaining("Older board"),
    ]);
    expect(getByText(document, "2 updates")).toBeTruthy();

    const menuItems = within(menu).getAllByRole("menuitem");
    menuItems[0].focus();
    menuItems[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(document.activeElement).toBe(menuItems[1]);
    menuItems[1].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(document.activeElement).toBe(menuItems[0]);

    within(menu).getByRole("menuitem", { name: /Newest board/ }).click();
    expect(window.location.hash).toBe("#/issues/bettr/2");
    expect(queryByText(document, "2 updates")).toBeNull();
  });
});
