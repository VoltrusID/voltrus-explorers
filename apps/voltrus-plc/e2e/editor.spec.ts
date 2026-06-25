import { test, expect } from "@playwright/test";

test.describe("Editor", () => {
  test("loads with default program content", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByText("PROGRAM Main")).toBeVisible({ timeout: 10000 });
  });

  test("toolbar renders build button", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("button", { name: /build/i })).toBeVisible({ timeout: 10000 });
  });

  test("toolbar renders run button", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("button", { name: /run/i })).toBeVisible({ timeout: 10000 });
  });

  test("sidebar shows watch tab", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByText("Watch")).toBeVisible({ timeout: 10000 });
  });

  test("undo and redo buttons present", async ({ page }) => {
    await page.goto("/");
    const undo = page.getByRole("button", { name: /undo/i });
    const redo = page.getByRole("button", { name: /redo/i });
    await expect(undo).toBeVisible({ timeout: 10000 });
    await expect(redo).toBeVisible();
  });
});
