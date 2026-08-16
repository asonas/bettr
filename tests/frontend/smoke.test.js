import { describe, expect, it } from "vitest";

import { kanbanColumns } from "../../src/web/state.js";

describe("frontend test environment", () => {
  it("loads the five configured Kanban columns", () => {
    expect(kanbanColumns).toEqual([
      ["todo", "Todo"],
      ["in_progress", "In progress"],
      ["blocked", "Blocked"],
      ["done", "Done"],
      ["cancelled", "Cancelled"],
    ]);
  });
});
