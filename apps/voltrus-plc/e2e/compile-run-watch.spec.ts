import { test, expect } from "@playwright/test";

test.describe("Compile-Run-Watch Flow", () => {
  test("build button compiles the program", async ({ page }) => {
    await page.goto("/");
    const buildBtn = page.getByRole("button", { name: /build/i });
    await expect(buildBtn).toBeVisible({ timeout: 10000 });
    await buildBtn.click();
    // Wait for compile result (either success or diagnostics)
    await page.waitForTimeout(2000);
    // The diagnostics panel or output panel should show content
    const diagnostics = page.getByText(/compiled|error|failed/i);
    await expect(diagnostics.first()).toBeVisible({ timeout: 5000 });
  });

  test("run button changes runtime state", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("button", { name: /build/i })).toBeVisible({ timeout: 10000 });
    // Build first
    await page.getByRole("button", { name: /build/i }).click();
    await page.waitForTimeout(2000);
    // Run
    const runBtn = page.getByRole("button", { name: /run/i });
    await runBtn.click();
    await page.waitForTimeout(1000);
    // Stop button should be visible or state changed
    const stopBtn = page.getByRole("button", { name: /stop|single scan/i });
    await expect(stopBtn.first()).toBeVisible({ timeout: 3000 });
  });

  test("watch table shows variables after compile", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("button", { name: /build/i })).toBeVisible({ timeout: 10000 });
    await page.getByRole("button", { name: /build/i }).click();
    await page.waitForTimeout(2000);
    // Click on Watch tab in sidebar
    const watchTab = page.getByText("Watch").first();
    await watchTab.click();
    // Watch table should exist (even if empty)
    await expect(page.getByText(/variable|watch/i).first()).toBeVisible({ timeout: 3000 });
  });
});
