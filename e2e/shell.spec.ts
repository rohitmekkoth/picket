import { expect, test } from "@playwright/test";

test("foundation renders, stays local, and supports keyboard interaction", async ({
  page,
  browserName,
}, testInfo) => {
  const failures: string[] = [];
  const remoteRequests: string[] = [];
  page.on("pageerror", (error) => failures.push(error.message));
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.hostname !== "127.0.0.1") remoteRequests.push(request.url());
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Browser preview" })).toBeVisible();
  for (const name of ["Request", "Claim", "Effect"]) {
    await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
  }
  await expect(page.getByText(/No accounts are connected/)).toBeVisible();
  const nextControl = browserName === "webkit" && process.platform === "darwin" ? "Alt+Tab" : "Tab";
  await page.keyboard.press(nextControl);
  await expect(page.getByRole("link", { name: "Picket home" })).toBeFocused();
  await page.keyboard.press(nextControl);
  await expect(page.getByRole("button", { name: "Check connection" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Check connection" })).toBeEnabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  expect(failures).toEqual([]);
  expect(remoteRequests).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath("foundation.png"), fullPage: true });
});
