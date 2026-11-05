import { expect, test } from "vitest";
import alertCopy from "./alert-copy.json";
import {
  action,
  coverageReason,
  coverageState,
  type IncidentSummary,
  level,
  notice,
  type Reason,
  reason,
  source,
} from "./api";
import contract from "./contract.json";
import { incidentTitle, reasonExplanation } from "./copy";

// contract.json is written by the core's ui_contract test from the Rust types, so a
// variant the core can send but these schemas would reject fails here, not at runtime.
test.each([
  ["action", action.options],
  ["source", source.options],
  ["coverageReason", coverageReason.options],
  ["coverageState", coverageState.options],
  ["level", level.options],
  ["notice", notice.options],
  ["reason", reason.options],
] as const)("the %s schema accepts exactly what the core sends", (name, options) => {
  expect([...options].sort()).toEqual([...contract[name]].sort());
});

// alert-copy.json is written by the same core test from the notification wording in Rust,
// so an alert and the screen it opens describe an incident in the same words.
test.each(alertCopy)("a $kind incident for $reason is titled as its notification", (item) => {
  const summary = {
    id: "incident/synthetic",
    severity: "review",
    status: "open",
    kind: item.kind,
    reasons: [item.reason],
    agent: item.agent,
    occurredAt: "1970-01-01T00:00:00Z",
    flaggedAt: "1970-01-01T00:00:00Z",
    subject: null,
    people: [],
  } as IncidentSummary;
  expect(incidentTitle(summary)).toBe(item.title);
  expect(reasonExplanation[item.reason as Reason]).toBe(item.explanation);
});
