import assert from "node:assert/strict";
import test from "node:test";

import { applyStatusUpdate, changedIssueKeys } from "../src/web/state.js";

const issue = (state, revision, updatedAt) => ({
  project: "bettr",
  issue: { number: 1, state, revision, updated_at: updatedAt, title: "Board" },
});

test("status updates record changed issues once and preserve the indicator", () => {
  const previous = { active: [issue("todo", 1, "2026-08-16T08:00:00Z")] };
  const next = { active: [issue("in_progress", 2, "2026-08-16T08:01:00Z")] };

  const update = applyStatusUpdate(previous, next, new Set());
  assert.deepEqual(Array.from(update.changedKeys), ["bettr#1"]);
  assert.deepEqual([...update.updatedIssues], ["bettr#1"]);

  const unchanged = applyStatusUpdate(next, next, update.updatedIssues);
  assert.deepEqual(Array.from(unchanged.changedKeys), []);
  assert.deepEqual([...unchanged.updatedIssues], ["bettr#1"]);
});

test("status updates detect an issue moving between status groups", () => {
  const previous = { active: [issue("in_progress", 2, "2026-08-16T08:01:00Z")] };
  const next = { recently_completed: [issue("done", 3, "2026-08-16T08:02:00Z")] };

  assert.deepEqual(Array.from(changedIssueKeys(previous, next)), ["bettr#1"]);
});
