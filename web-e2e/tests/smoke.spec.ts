import { expect, test } from "@playwright/test";

// React UI smoke tests. The legacy vanilla UI has been removed; these tests
// drive the React SPA served at `/` and exercise the zustand store + WS
// connection + tab switching.

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    try {
      localStorage.removeItem("unistar-web-theme");
    } catch {
      /* ignore */
    }
  });
});

test("home page loads with React root", async ({ page }) => {
  await page.goto("/");
  // The React app renders the brand text in the topbar.
  await expect(page.locator("header")).toContainText("unistar-coworker");
  // The theme toggle button is present (lucide icon).
  await expect(page.locator("header button[aria-label*='mode']")).toBeVisible();
  // The main panel is present.
  await expect(page.locator("#main")).toBeVisible();
});

test("theme toggle switches class on html", async ({ page }) => {
  await page.goto("/");
  // Wait for the WS to connect and the React app to mount the theme button.
  await expect(page.locator("header button[aria-label*='mode']")).toBeVisible({
    timeout: 15_000,
  });

  const html = page.locator("html");
  const before =
    (await html.getAttribute("class"))?.includes("dark") ||
    (await html.getAttribute("data-theme")) === "dark"
      ? "dark"
      : "light";
  await page.click("header button[aria-label*='mode']");
  const classAfter = (await html.getAttribute("class")) || "";
  const dataThemeAfter = await html.getAttribute("data-theme");
  const after = classAfter.includes("dark") ? "dark" : dataThemeAfter || "light";
  expect(after).not.toBe(before);
  expect(["light", "dark"]).toContain(after);
});

test("approvals tab renders pending empty state", async ({ page }) => {
  await page.goto("/");
  // Wait for the tab list to render (WS snapshot arrives).
  await expect(page.locator("header")).toContainText("unistar-coworker", {
    timeout: 15_000,
  });
  // Click the Approvals tab trigger.
  const approvalsTab = page.getByRole("tab", { name: /approvals/i });
  await expect(approvalsTab).toBeVisible({ timeout: 10_000 });
  await approvalsTab.click();
  // The Approvals tab shows a "Pending" sub-tab and an empty-state message.
  await expect(page.getByText(/no pending approvals/i)).toBeVisible({
    timeout: 10_000,
  });
});
