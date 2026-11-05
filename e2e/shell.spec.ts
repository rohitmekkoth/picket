import { expect, test } from "@playwright/test";

test("the browser preview stays local, says it needs the app, and supports the keyboard", async ({
  page,
  browserName,
}, testInfo) => {
  // Treat console exceptions and non-local requests as end-user failures.
  const failures: string[] = [];
  const remoteRequests: string[] = [];
  page.on("pageerror", (error) => failures.push(error.message));
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.hostname !== "127.0.0.1") remoteRequests.push(request.url());
  });
  await page.goto("/");
  await expect(page.getByText("Open the desktop app")).toBeVisible();
  for (const name of ["Overview", "Review", "Activity", "Rules", "Settings"]) {
    await expect(page.getByRole("link", { name, exact: true })).toBeVisible();
  }
  // Safari uses Option-Tab for links when full keyboard access is disabled.
  const nextControl = browserName === "webkit" && process.platform === "darwin" ? "Alt+Tab" : "Tab";
  await page.keyboard.press(nextControl);
  await expect(page.getByRole("link", { name: "Picket overview" })).toBeFocused();
  await page.keyboard.press(nextControl);
  await expect(page.getByRole("link", { name: "Overview", exact: true })).toBeFocused();
  await page.keyboard.press(nextControl);
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/review$/);
  await expect(page.getByRole("link", { name: "Review", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  expect(failures).toEqual([]);
  expect(remoteRequests).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath("preview.png"), fullPage: true });
});

test("notification actions select a visible test result", async ({ page }) => {
  await page.goto("/#/notification-test/review");
  await expect(page.getByRole("heading", { name: "Review action received" })).toBeVisible();
  await page.evaluate(() => {
    window.location.hash = "#/notification-test/expected";
  });
  await expect(
    page.getByRole("heading", { name: "This was expected action received" }),
  ).toBeVisible();
});
