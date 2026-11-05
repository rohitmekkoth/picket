import { expect, test } from "vitest";
import { action, coverageReason, coverageState, level, notice, reason, source } from "./api";
import contract from "./contract.json";

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
