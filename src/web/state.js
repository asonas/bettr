(() => {
  "use strict";

  const kanbanColumns = [
    ["todo", "Todo"],
    ["in_progress", "In progress"],
    ["blocked", "Blocked"],
    ["done", "Done"],
    ["cancelled", "Cancelled"],
  ];

  function issueMap(status) {
    return Object.values(status || {}).flatMap((items) => Array.isArray(items) ? items : []).reduce((map, item) => {
      const issue = item.issue || item;
      const project = item.project || issue.project || "";
      if (project && issue.number != null) map.set(`${project}#${issue.number}`, { project, issue });
      return map;
    }, new Map());
  }

  function allIssues(status) { return [...issueMap(status).values()]; }

  function changedIssueKeys(previous, next) {
    const before = issueMap(previous);
    const after = issueMap(next);
    const keys = new Set([...before.keys(), ...after.keys()]);
    return [...keys].filter((key) => {
      const beforeIssue = before.get(key)?.issue;
      const afterIssue = after.get(key)?.issue;
      if (!beforeIssue || !afterIssue) return true;
      return `${beforeIssue.updated_at}:${beforeIssue.revision}:${beforeIssue.state}` !== `${afterIssue.updated_at}:${afterIssue.revision}:${afterIssue.state}`;
    });
  }

  function applyStatusUpdate(previous, next, updatedIssues) {
    const changedKeys = previous ? changedIssueKeys(previous, next) : [];
    const nextUpdatedIssues = new Set(updatedIssues || []);
    changedKeys.forEach((key) => nextUpdatedIssues.add(key));
    return { changedKeys, updatedIssues: nextUpdatedIssues };
  }

  globalThis.BettrWebState = Object.freeze({ kanbanColumns, issueMap, allIssues, changedIssueKeys, applyStatusUpdate });
})();
