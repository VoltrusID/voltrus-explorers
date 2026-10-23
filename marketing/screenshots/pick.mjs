// Picks a choice of a SheetSelect from a script. On desktop and iPad it is a trigger button (`.vt-select-trigger`) that opens a
// Radix Select or, with more than five options, a Combobox; on the iPhone layout it is a button that opens a bottom sheet. Both are
// chosen by the row's label.
//   current: what the control shows now ("New profile") or its aria-label; label: the row's title.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function pickOption(page, current, label) {
  const desktop = await page.evaluateHandle((current) => [...document.querySelectorAll("button.vt-select-trigger")].find((el) => el.textContent.trim() === current || el.getAttribute("aria-label") === current) ?? null, current);
  const trigger = desktop.asElement();
  if (trigger) {
    await trigger.click();
    await page.waitForSelector('[role="option"]', { timeout: 5000 });
    await sleep(250);
    const row = await page.evaluateHandle((label) => [...document.querySelectorAll('[role="option"]')].find((el) => el.querySelector(".vt-option__label")?.textContent === label) ?? null, label);
    const el = row.asElement();
    if (!el) throw new Error(`no option "${label}"`);
    await el.click();
    await sleep(350);
    return;
  }
  const opened = await page.evaluate((current) => {
    const b = [...document.querySelectorAll("button[data-sheet-select]")].find((el) => el.textContent.trim() === current);
    if (!b) return false;
    b.click();
    return true;
  }, current);
  if (!opened) throw new Error(`no select showing "${current}"`);
  await page.waitForSelector(".vm-sheet-list", { timeout: 5000 });
  await sleep(350);
  const chose = await page.evaluate((label) => {
    const row = [...document.querySelectorAll('.vm-sheet-list [role="option"]')].find((el) => el.querySelector(".vm-row__title")?.textContent === label);
    if (!row) return false;
    row.click();
    return true;
  }, label);
  if (!chose) throw new Error(`no sheet row "${label}"`);
  await sleep(400);
}
