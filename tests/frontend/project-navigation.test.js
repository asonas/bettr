import { getByRole, queryByText } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { jsonResponse, mountShell } from "./support.js";

describe("project navigation", () => {
  it("recovers when a refresh fails after the initial project list loaded", async () => {
    mountShell();
    const projects = [
      { name: "bettr", archived: false },
      { name: "beacon", archived: false },
    ];
    const fetch = vi.fn()
      .mockResolvedValueOnce(jsonResponse(projects))
      .mockRejectedValueOnce(new Error("temporary failure"))
      .mockResolvedValueOnce(jsonResponse(projects));
    const controller = createWebController({ fetch });

    await controller.loadProjectNavigation();
    const link = getByRole(document, "link", { name: "Project bettr" });
    expect(link).toBeTruthy();
    expect(link.querySelector(".project-nav-dot").textContent).toBe("B1");
    expect(getByRole(document, "link", { name: "Project beacon" }).querySelector(".project-nav-dot").textContent).toBe("B2");

    await controller.loadProjectNavigation();
    expect(getByRole(document, "link", { name: "Project bettr" })).toBeTruthy();
    expect(queryByText(document, "Projects unavailable")).toBeNull();

    await controller.loadProjectNavigation();
    expect(getByRole(document, "link", { name: "Project bettr" })).toBeTruthy();
    expect(queryByText(document, "Projects unavailable")).toBeNull();
  });
});
