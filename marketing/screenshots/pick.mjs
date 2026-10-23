// Picks a choice of a SheetSelect from a script. On desktop and iPad it is a native <select> (chosen by value); on the iPhone
// layout it is a button that opens a bottom sheet (chosen by the row's label).
//   current: what the control shows now ("New profile"); value: the <option> value; label: the sheet row's title (default value).
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function pickOption(page, current, value, label = value) {
  const native = await page.evaluate((current) => {
    const s = [...document.querySelectorAll("select")].find((el) => [...el.options].some((o) => o.textContent === current));
    if (!s) return false;
    s.setAttribute("data-pick", "1");
    return true;
  }, current);
  if (native) {
    await page.select("[data-pick]", value);
    await page.evaluate(() => document.querySelector("[data-pick]")?.removeAttribute("data-pick"));
    await sleep(300);
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
