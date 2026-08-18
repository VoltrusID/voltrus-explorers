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
    await expect(page.getByRole("button", { name: "Run", exact: true })).toBeVisible({ timeout: 10000 });
  });

  test("sidebar shows watch tab", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("button", { name: "Watch", exact: true })).toBeVisible({ timeout: 10000 });
  });

  test("undo and redo buttons present", async ({ page }) => {
    await page.goto("/");
    const undo = page.getByTitle("Undo (Cmd+Z)");
    const redo = page.getByTitle("Redo (Cmd+Shift+Z)");
    await expect(undo).toBeVisible({ timeout: 10000 });
    await expect(redo).toBeVisible();
  });
});
