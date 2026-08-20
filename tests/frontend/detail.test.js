import { getByText, queryByText } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { issue, jsonResponse, mountShell } from "./support.js";

describe("Issue detail activity", () => {
  it("renders meaningful activity text for creation, edits, and comments", async () => {
    mountShell();
    const history = [
      { event_type: "issue_created", metadata: {}, context: { agent: "codex" }, created_at: "2026-08-16T08:00:00Z" },
      { event_type: "issue_updated", metadata: { changes: { title: { from: "Old", to: "New" } } }, context: { agent: "codex" }, created_at: "2026-08-16T08:01:00Z" },
      { event_type: "comment_added", metadata: { body: "Human review requested" }, context: { kind: "human" }, created_at: "2026-08-16T08:02:00Z" },
    ];
    const fetch = vi.fn((path) => path.startsWith("/api/issues/")
      ? Promise.resolve(jsonResponse({ issue: issue({ title: "New" }), history }))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    window.location.hash = "#/issues/bettr/1";
    await controller.route();

    expect(getByText(document, "Issue created")).toBeTruthy();
    expect(getByText(document, "Changed: title")).toBeTruthy();
    expect(getByText(document, "Human review requested")).toBeTruthy();
    expect(queryByText(document, "Change recorded")).toBeNull();
  });

  it("turns escaped line breaks in comments into readable paragraphs", async () => {
    mountShell();
    const history = [
      { event_type: "comment_added", metadata: { body: "First paragraph\\n\\nSecond paragraph" }, context: { kind: "human" }, created_at: "2026-08-20T09:00:00Z" },
    ];
    const fetch = vi.fn((path) => path.startsWith("/api/issues/")
      ? Promise.resolve(jsonResponse({ issue: issue(), history }))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    window.location.hash = "#/issues/bettr/1";
    await controller.route();

    const activityBody = document.querySelector(".activity-body");
    expect(activityBody.textContent).toBe("First paragraph\n\nSecond paragraph");
    expect(activityBody.innerHTML).not.toContain("\\n");
  });
});
