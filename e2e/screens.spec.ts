import { expect, type Page, test } from "@playwright/test";
import { calls, mockDesktop } from "./desktop";

// The product screens against a synthetic core. Every test also checks that nothing
// leaves the machine, nothing throws, and the window never scrolls sideways.

function guard(page: Page) {
  const problems: string[] = [];
  page.on("pageerror", (error) => problems.push(error.message));
  page.on("request", (request) => {
    if (new URL(request.url()).hostname !== "127.0.0.1") problems.push(request.url());
  });
  return problems;
}

async function settle(
  page: Page,
  name: string,
  testInfo: { outputPath: (name: string) => string },
) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.screenshot({ path: testInfo.outputPath(`${name}.png`), fullPage: true });
}

test("the overview answers whether everything is OK", async ({ page }, testInfo) => {
  const problems = guard(page);
  await mockDesktop(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "2 things need a look" })).toBeVisible();
  await expect(
    page.getByText(
      "Instinct did 10 things today. 7 matched what you asked, and for 1 Picket couldn’t tell.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: /Instinct emailed someone you didn’t mention/ }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "What Picket can see" })).toBeVisible();
  await expect(page.getByText("Your chat with Instinct")).toBeVisible();
  await expect(page.getByText("Waiting for the on-device model")).toBeVisible();
  await settle(page, "overview", testInfo);
  expect(problems).toEqual([]);
});

test("an incident tells the story and records the decision", async ({ page }, testInfo) => {
  const problems = guard(page);
  await mockDesktop(page);
  await page.goto("/#/incident/aaa111");
  await expect(
    page.getByRole("heading", { level: 1, name: "Instinct emailed someone you didn’t mention" }),
  ).toBeVisible();
  await expect(page.getByText("“Email Sam about lunch Thursday”").first()).toBeVisible();
  await expect(page.getByText("“Sent!”")).toBeVisible();
  await expect(page.getByText("Not in your request")).toBeVisible();
  await settle(page, "incident", testInfo);
  await page.getByRole("button", { name: "This wasn’t OK" }).click();
  await expect(page.getByRole("heading", { name: "What you can do now" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Mark revoked" })).toBeVisible();
  await settle(page, "incident-not-ok", testInfo);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("button", { name: "This was fine" })).toBeVisible();
  await page.getByRole("button", { name: "This was fine" }).click();
  await expect(page.getByText(/Marked as fine/).first()).toBeVisible();
  const decisions = (await calls(page))
    .filter((call) => call.cmd === "resolve_incident")
    .map((call) => call.args.decision);
  expect(decisions).toEqual(["not_ok", "undo", "fine"]);
  expect(problems).toEqual([]);
});

test("rules use one control and boundaries read as sentences", async ({ page }, testInfo) => {
  const problems = guard(page);
  await mockDesktop(page);
  await page.goto("/#/rules");
  await expect(
    page.getByRole("heading", { name: "When Instinct doesn’t do what you asked" }),
  ).toBeVisible();
  const row = page.getByRole("group", { name: "Sign-in or verification codes" });
  await row.getByRole("button", { name: "Review" }).click();
  await expect(row.getByRole("button", { name: "Review" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Never email anyone at bank.example")).toBeVisible();
  await settle(page, "rules", testInfo);
  await page.getByRole("link", { name: "Add a boundary" }).click();
  await expect(page.getByRole("heading", { name: "Add a boundary" })).toBeVisible();
  await page.getByLabel("People or companies").fill("bank.example");
  await expect(page.getByText("Would have flagged 1")).toBeVisible();
  await expect(page.getByText("“Statement question” to help@bank.example")).toBeVisible();
  await settle(page, "boundary", testInfo);
  await page.getByRole("button", { name: "Add boundary" }).click();
  await expect(page).toHaveURL(/#\/rules$/);
  expect(problems).toEqual([]);
});

test("activity lists everything with honest gaps", async ({ page }, testInfo) => {
  const problems = guard(page);
  await mockDesktop(page);
  await page.goto("/#/activity");
  await expect(page.getByRole("heading", { name: "Activity" })).toBeVisible();
  await expect(page.getByText("Sent “Lunch Thursday?” to Sam Lee and Jordan Park")).toBeVisible();
  await expect(page.getByText(/own address, which Picket can’t see/)).toBeVisible();
  await expect(page.getByText("Picket lost sight of Gmail")).toBeVisible();
  await settle(page, "activity", testInfo);
  expect(problems).toEqual([]);
});

test("settings keep the connection, agents and model together", async ({ page }, testInfo) => {
  const problems = guard(page);
  await mockDesktop(page);
  await page.goto("/#/settings");
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.getByText("Connected", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Add another agent" }).click();
  await expect(page.getByLabel("Agent name")).toBeVisible();
  await expect(page.getByText("iMessage agent · +15550104477")).toBeVisible();
  await settle(page, "settings", testInfo);
  expect(problems).toEqual([]);
});
