// @vitest-environment node
import { expect, test } from "vitest";
import { observe, tokenize } from "./gog.js";

const exec = (command: string, result?: unknown, extra: Record<string, unknown> = {}) =>
  observe("exec", { command, ...extra }, result, undefined);

test("a plain send reports its recipients and the sent message id", () => {
  const observed = exec(
    "gog gmail send --to 'Sam <Sam@Example.test>',lee@example.test --cc=kim@example.test --subject 'Hi there' --body 'Private text'",
    "message_id\t18f0a1b2c3d4e5f6\nthread_id\t18f0a1b2c3d4e5f0\n",
  );
  expect(observed).toEqual({
    kind: "report",
    report: {
      action: "email_send",
      targets: ["sam@example.test", "lee@example.test", "kim@example.test"],
      resource: "18f0a1b2c3d4e5f6",
      outcome: "succeeded",
    },
  });
  expect(JSON.stringify(observed)).not.toMatch(/Private|Hi there/);
});

test("JSON output, the root alias and the account flag are understood", () => {
  expect(
    exec(
      "gog -a Owner@Example.test --json send --to sam@example.test --body x",
      '{"messageId":"18f0a1b2c3d4e5f6","threadId":"t"}',
    ),
  ).toEqual({
    kind: "report",
    report: {
      action: "email_send",
      account: "owner@example.test",
      targets: ["sam@example.test"],
      resource: "18f0a1b2c3d4e5f6",
      outcome: "succeeded",
    },
  });
  const viaEnv = exec("GOG_ACCOUNT=owner@example.test gog mail send --to a@example.test", "");
  expect(viaEnv.kind === "report" && viaEnv.report.account).toBe("owner@example.test");
});

test("without an id in the output the outcome stays unknown, or failed on an error", () => {
  const unknown = exec("gog gmail send --to sam@example.test", "Error: quota exceeded");
  expect(unknown.kind === "report" && unknown.report.outcome).toBe("unknown");
  expect(unknown.kind === "report" && unknown.report.resource).toBeUndefined();
  const failed = observe("exec", { command: "gog gmail send --to a@example.test" }, "", "exit 1");
  expect(failed.kind === "report" && failed.report.outcome).toBe("failed");
  const batch = exec(
    "gog gmail send --to a@example.test,b@example.test",
    "to\ta@example.test\nmessage_id\tm1\n\nto\tb@example.test\nmessage_id\tm2\n",
  );
  expect(batch.kind === "report" && batch.report.resource).toBeUndefined();
});

test("calendar writes report the event id from gog's output", () => {
  expect(
    exec(
      "gog calendar create primary --summary Review --from 2026-11-03T10:00:00Z --to 2026-11-03T11:00:00Z --attendees 'sam@example.test;optional,lee@example.test'",
      "id\tabc123def456\nsummary\tReview\n",
    ),
  ).toEqual({
    kind: "report",
    report: {
      action: "calendar_create",
      targets: ["sam@example.test", "lee@example.test"],
      resource: "abc123def456",
      outcome: "succeeded",
    },
  });
  const deleted = exec(
    "gog cal rm primary abc123def456 -y",
    "deleted\ttrue\ncalendarId\tprimary\neventId\tabc123def456\n",
  );
  expect(deleted.kind === "report" && deleted.report).toMatchObject({
    action: "calendar_delete",
    resource: "abc123def456",
  });
  const rsvp = exec("gog calendar rsvp primary abc123def456 --status accepted", {
    content: [{ type: "text", text: "id\tabc123def456" }],
  });
  expect(rsvp.kind === "report" && rsvp.report.outcome).toBe("succeeded");
});

test("reads, dry runs and other tools are ignored", () => {
  expect(exec("gog gmail search 'newer_than:7d' --max 10", "message_id\tx")).toEqual({
    kind: "ignored",
  });
  expect(exec("gog gmail send --to a@example.test --dry-run")).toEqual({ kind: "ignored" });
  expect(exec("ls -la")).toEqual({ kind: "ignored" });
  expect(exec("gog drive ls")).toEqual({ kind: "ignored" });
  expect(observe("browser", { command: "gog gmail send" }, "", undefined)).toEqual({
    kind: "ignored",
  });
});

test("gog commands Picket cannot interpret are counted, never guessed", () => {
  for (const command of [
    "gog gmail send --to a@example.test && echo done",
    "cat list | xargs gog gmail send --to",
    "gog gmail send --to $(cat recipients)",
    "sudo gog gmail send --to a@example.test",
    "gog api POST gmail/v1/users/me/messages/send",
    "gog mcp",
    "gog gmail autoreply --query x",
    "gog calendar move primary e1 other",
    "gog --unknown-flag gmail send --to a@example.test",
    "gog gmail send --to 'unterminated",
  ]) {
    expect(exec(command), command).toEqual({ kind: "unrecognized" });
  }
});

test("the tokenizer handles quoting and refuses expansion", () => {
  expect(tokenize(`gog send --subject "a \\"b\\" c" 'd e' f\\ g`)).toEqual([
    "gog",
    "send",
    "--subject",
    'a "b" c',
    "d e",
    "f g",
  ]);
  expect(tokenize('gog send --to "$HOME"')).toBeNull();
  expect(tokenize("gog send; rm -rf x")).toBeNull();
  expect(tokenize("gog send\nrm x")).toBeNull();
});
