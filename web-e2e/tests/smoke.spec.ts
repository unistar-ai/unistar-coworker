import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    try {
      localStorage.removeItem("unistar-web-theme");
    } catch {
      /* ignore */
    }
  });
});

test("home page loads", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".brand")).toContainText("unistar-coworker");
  await expect(page.locator("#theme-toggle")).toBeVisible();
  await expect(page.locator("#main")).toBeVisible();
});

test("theme toggle switches data-theme on html", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(
    () => document.getElementById("conn-dot")?.classList.contains("live"),
    null,
    { timeout: 15_000 },
  );

  const html = page.locator("html");
  const before = (await html.getAttribute("data-theme")) || "dark";
  await page.click("#theme-toggle");
  const after = await html.getAttribute("data-theme");
  expect(after).toBeTruthy();
  expect(after).not.toBe(before);
  expect(["light", "dark"]).toContain(after);
});

test("approval dialog shows tool command payload", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(
    () => typeof updateApprovalModal === "function",
    null,
    { timeout: 15_000 },
  );

  await page.evaluate(() => {
    // Playwright runs in the page realm; `state` / `updateApprovalModal` come from app.js.
    // @ts-expect-error page globals from static scripts
    state.approval_dialog = {
      id: "e2e-approval-1",
      tool_name: "bash_run",
      description: "Approve bash_run for smoke test",
      tool_args_json: JSON.stringify({
        command: "echo smoke-test",
        workdir: "/tmp",
      }),
      deciding: false,
      approve_armed: true,
      approve_arm_ms_remaining: 0,
    };
    // @ts-expect-error page globals from static scripts
    updateApprovalModal();
  });

  const modal = page.locator(".approval-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".approval-tool-name")).toHaveText("bash_run");
  await expect(modal.locator(".approval-payload-pre")).toContainText(
    "echo smoke-test",
  );
  await expect(modal.getByRole("button", { name: "Approve" })).toBeVisible();
});
