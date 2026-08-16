import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { jsonResponse, mountShell } from "./support.js";

describe("navigation controls", () => {
  it("does not install the removed keyboard shortcut handler", () => {
    mountShell();
    const fetch = vi.fn((path) => Promise.resolve(jsonResponse(path === "/api/projects" ? [] : {
      attention: [], stale: [], blocked: [], recently_completed: [], active: [],
    })));
    const controller = createWebController({ fetch });
    controller.start();
    const before = window.location.hash;

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "g" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "o" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "/" }));

    expect(window.location.hash).toBe(before);
    controller.stop();
  });
});
