import { fireEvent, getAllByLabelText, getAllByRole, getByLabelText, getByRole, getByText } from "@testing-library/dom";
import { describe, expect, it, vi } from "vitest";

import { createWebController } from "../../src/web/app.js";
import { issue, jsonResponse, mountShell } from "./support.js";

function detail({ decisions = [], wait = null, state = "blocked", revision = 2 } = {}) {
  return {
    project: "bettr",
    issue: issue({ state, revision }),
    history: [],
    decisions,
    dependencies: [],
    wait,
  };
}

function decision(id, question) {
  return {
    id,
    issue: "bettr#1",
    question,
    background: "The answer changes the rollout order.",
    requester_kind: "agent",
    requester_name: "codex",
    requester_session_id: "session-a",
    status: "open",
    answer: null,
    resolver_kind: null,
    resolver_name: null,
    resolver_session_id: null,
    created_at: "2026-08-20T08:00:00Z",
    resolved_at: null,
  };
}

describe("human decision UI", () => {
  it("shows wait context and accessible forms for multiple unresolved decisions", async () => {
    mountShell();
    const decisions = [
      decision("11111111-1111-4111-8111-111111111111", "Which parser should be used?"),
      decision("22222222-2222-4222-8222-222222222222", "Which rollout should be used?"),
    ];
    const fetch = vi.fn((path) => path.startsWith("/api/issues/")
      ? Promise.resolve(jsonResponse(detail({
        decisions,
        wait: { kind: "human", label: "Human decision", reason: "A human decision is required" },
      })))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    window.location.hash = "#/issues/bettr/1";
    await controller.route();

    expect(getByRole(document, "heading", { name: "Waiting for a decision" })).toBeTruthy();
    expect(document.querySelector(".wait-kind").textContent).toBe("Human decision");
    expect(getByText(document, "A human decision is required")).toBeTruthy();
    expect(getAllByRole(document, "form")).toHaveLength(2);
    expect(getAllByLabelText(document, /^Answer/)).toHaveLength(2);
    expect(getAllByLabelText(document, /^Next state/)).toHaveLength(2);
    expect(getByText(document, "Which parser should be used?")).toBeTruthy();
    expect(getByText(document, "Which rollout should be used?")).toBeTruthy();
  });

  it("switches metadata requirements with the selected next state", async () => {
    mountShell();
    const unresolved = decision("11111111-1111-4111-8111-111111111111", "Choose a state");
    const fetch = vi.fn((path) => path.startsWith("/api/issues/")
      ? Promise.resolve(jsonResponse(detail({ decisions: [unresolved] })))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    window.location.hash = "#/issues/bettr/1";
    await controller.route();

    const form = getByRole(document, "form");
    const nextState = getByLabelText(form, "Next state");
    const summary = getByLabelText(form, /^Summary/);
    const verification = getByLabelText(form, /^Verification/);
    const reason = getByLabelText(form, /^Blocked reason/);
    const waitKind = getByLabelText(form, /^Wait kind/);
    expect(summary).toHaveProperty("disabled", true);
    expect(reason).toHaveProperty("disabled", true);

    fireEvent.change(nextState, { target: { value: "done" } });
    expect(summary).toHaveProperty("required", true);
    expect(verification).toHaveProperty("required", true);
    expect(reason).toHaveProperty("disabled", true);
    expect(waitKind).toHaveProperty("disabled", true);

    fireEvent.change(nextState, { target: { value: "blocked" } });
    expect(reason).toHaveProperty("required", true);
    expect(waitKind).toHaveProperty("required", true);
    expect(summary).toHaveProperty("disabled", true);
  });

  it("sends one resolve request while a form is in flight and refreshes after success", async () => {
    mountShell();
    const unresolved = decision("11111111-1111-4111-8111-111111111111", "Choose a parser");
    const resolved = { ...unresolved, status: "resolved", answer: "Use parser A" };
    const initial = detail({ decisions: [unresolved] });
    const afterResolve = detail({ decisions: [resolved], state: "todo", revision: 3, wait: null });
    const refreshedStatus = { attention: [], stale: [], blocked: [], recently_completed: [], active: [{ ...afterResolve.issue, project: "bettr" }] };
    const fetch = vi.fn((path, options = {}) => {
      if (options.method === "POST") return Promise.resolve(jsonResponse(resolved));
      if (path === "/api/status") return Promise.resolve(jsonResponse(refreshedStatus));
      return Promise.resolve(jsonResponse(fetch.mock.calls.some(([, call]) => call?.method === "POST") ? afterResolve : initial));
    });
    const controller = createWebController({ fetch });

    window.location.hash = "#/issues/bettr/1";
    await controller.route();
    const form = getByRole(document, "form");
    fireEvent.input(getByLabelText(form, /^Answer/), { target: { value: "Use parser A" } });
    fireEvent.submit(form);
    fireEvent.submit(form);

    const postCalls = fetch.mock.calls.filter(([, options]) => options?.method === "POST");
    expect(postCalls).toHaveLength(1);
    expect(postCalls[0][1].body).toContain('"expected_revision":2');
    expect(postCalls[0][1].body).toContain('"next_state":"todo"');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.querySelector(".decision-record-header strong").textContent).toBe("Decision resolved");
    expect(getByText(document, "No unresolved decisions")).toBeTruthy();
    expect(controller.state).toBeTruthy();
  });

  it("renders a useful empty state when no human decision is unresolved", async () => {
    mountShell();
    const fetch = vi.fn((path) => path.startsWith("/api/issues/")
      ? Promise.resolve(jsonResponse(detail({ decisions: [], state: "todo", revision: 3 })))
      : Promise.resolve(jsonResponse([])));
    const controller = createWebController({ fetch });

    window.location.hash = "#/issues/bettr/1";
    await controller.route();

    expect(getByRole(document, "heading", { name: "Decision" })).toBeTruthy();
    expect(getByText(document, "No unresolved decisions")).toBeTruthy();
    expect(getByText(document, "No human decision is waiting for an answer.")).toBeTruthy();
    expect(document.querySelector("form")).toBeNull();
  });
});
