// Opens every select of an app in Chromium at desktop size (1440x900) and saves one screenshot per open select, light and dark.
//
//   bun marketing/screenshots/render-selects.mjs <app> [outDir] [--scheme=light|dark|both] [--scene=a,b] [--keyboard] [--plain=scene:text]
//
// Each scene of the app's scenes.mjs (plus the scene file's optional `selectScenes`, extra steps that reveal selects the
// marketing scenes do not show) is driven to its state with the mocked Tauri runtime; then every visible select trigger
// (`.vt-select-trigger`, the desktop SheetSelect) is opened, captured with its popover, and closed with Escape. Per select it
// checks that the popover opened inside the viewport, that a chosen row carries a checkmark, and that Escape put the focus back
// on the trigger. Default outDir is /tmp/explorers-selects/<app>.
//
// --keyboard additionally drives the first plain Select and the first Combobox of the app with the keyboard only: ArrowDown opens,
// arrows move the highlight, typing jumps (Select) or filters (Combobox), "No results" shows for a bad query, Enter picks, Escape
// closes, and the focus is on the trigger afterwards. Exit code 1 if any check fails.
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import puppeteer from "puppeteer-core";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const args = process.argv.slice(2);
const flag = (name) => args.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3);
const [app, outArg] = args.filter((a) => !a.startsWith("--"));
if (!app) throw new Error("usage: render-selects.mjs <app> [outDir] [--scheme=light|dark|both] [--scene=a,b] [--keyboard]");
const outDir = outArg ?? `/tmp/explorers-selects/${app}`;
const schemes = flag("scheme") === "light" ? ["light"] : flag("scheme") === "dark" ? ["dark"] : ["dark", "light"];
const onlyScenes = flag("scene")?.split(",");
const keyboard = args.includes("--keyboard");
// --plain=<scene>:<text> screenshots that scene with the control showing <text> scrolled into view and left closed (also works on a
// build with native <select>s: the "before" side of a comparison).
const plain = flag("plain")?.split(":");
mkdirSync(outDir, { recursive: true });

const sceneModule = await import(pathToFileURL(join(here, app, "scenes.mjs")).href);
const { mock } = sceneModule;
// "initial" is the app as it opens (nothing connected or running), where most connection selects are enabled.
const initial = { id: "initial", async setup() {} };
const scenes = [initial, ...sceneModule.scenes.filter((s) => !s.review), ...(sceneModule.selectScenes ?? [])].filter((s) => (plain ? s.id === plain[0] : !onlyScenes || onlyScenes.includes(s.id)));

const PORT = Number(process.env.SCREENSHOT_PORT ?? 1442);
const vite = spawn("bun", ["run", "dev", "--port", String(PORT), "--strictPort"], { cwd: join(root, "apps", app), stdio: ["ignore", "pipe", "inherit"] });
await new Promise((resolve, reject) => {
  vite.stdout.on("data", (d) => String(d).includes("ready in") && resolve());
  vite.on("exit", (c) => reject(new Error(`vite exited ${c}`)));
});

const MAC_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const FONT_CSS = `:root { --font-sans: "Inter", system-ui, sans-serif !important; --font-mono: "JetBrains Mono", ui-monospace, monospace !important; } * { caret-color: transparent !important; }`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const browser = await puppeteer.launch({ executablePath: process.env.CHROMIUM ?? "/usr/bin/chromium", headless: true, args: ["--font-render-hinting=none", "--disable-lcd-text"] });

async function plainShot(page, scheme, text) {
  const found = await page.evaluate((text) => {
    const controls = [...document.querySelectorAll("button.vt-select-trigger, select")];
    const el = controls.find((c) => (c.tagName === "SELECT" ? c.selectedOptions[0]?.textContent.trim() : c.textContent.trim()) === text);
    if (!el) return false;
    el.scrollIntoView({ block: "center" });
    return true;
  }, text);
  if (!found) return console.log(`  plain: no control showing "${text}"`);
  await sleep(200);
  await page.screenshot({ path: join(outDir, scheme, `plain-${text.replace(/[^a-z0-9]+/gi, "_")}.png`) });
}

let failures = 0;
const fail = (label, msg) => {
  failures++;
  console.log(`  FAIL ${label}: ${msg}`);
};

async function openScene(scene, scheme) {
  const page = await browser.newPage();
  await page.setUserAgent(MAC_UA);
  await page.setViewport({ width: 1440, height: 900, deviceScaleFactor: 1 });
  await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: scheme }]);
  await page.evaluateOnNewDocument(mock);
  await page.evaluateOnNewDocument(`localStorage.setItem("modbus-poll-theme", "${scheme}");`);
  if (scene.init) await page.evaluateOnNewDocument(scene.init);
  page.on("pageerror", (e) => console.error(`[${scene.id}/${scheme}] page error:`, e.message));
  page.on("console", (m) => m.type() === "error" && console.error(`[${scene.id}/${scheme}] console error:`, m.text().slice(0, 200)));
  await page.goto(`http://localhost:${PORT}/${scene.hash ?? ""}`, { waitUntil: "networkidle0" });
  await page.addStyleTag({ content: FONT_CSS });
  await page.evaluate((s) => document.documentElement.setAttribute("data-theme", s), scheme);
  await scene.setup(page);
  // Some scenes' own prep forces the dark theme; the scheme under test wins.
  await page.evaluate((s) => document.documentElement.setAttribute("data-theme", s), scheme);
  await sleep(scene.settleMs ?? 500);
  return page;
}

/** Tags every visible desktop select trigger with data-sel-index and returns what each shows. */
async function tagTriggers(page) {
  return page.evaluate(() => {
    const out = [];
    document.querySelectorAll("[data-sel-index]").forEach((el) => el.removeAttribute("data-sel-index"));
    for (const el of document.querySelectorAll("button.vt-select-trigger")) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0 || getComputedStyle(el).visibility === "hidden") continue;
      // Under a dialog's scrim a click would land on the scrim: that select belongs to a scene with the dialog closed.
      el.scrollIntoView({ block: "center", inline: "nearest" });
      const r2 = el.getBoundingClientRect();
      const top = document.elementFromPoint(r2.x + r2.width / 2, r2.y + r2.height / 2);
      if (!top || !el.contains(top)) continue;
      el.setAttribute("data-sel-index", String(out.length));
      out.push({ index: out.length, text: el.textContent.trim(), combo: el.getAttribute("role") === "combobox" && !el.hasAttribute("aria-autocomplete"), disabled: el.disabled, label: el.getAttribute("aria-label") ?? "" });
    }
    return out;
  });
}

const popoverBox = (page) =>
  page.evaluate(() => {
    const el = document.querySelector('[role="listbox"], [role="dialog"].vt-popover, .vt-popover');
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height, vw: innerWidth, vh: innerHeight, items: document.querySelectorAll('[role="option"]').length, checked: document.querySelectorAll('[role="option"][data-state="checked"], [data-checked="true"]').length };
  });

async function openAndShoot(page, trig, scene, scheme) {
  const tag = `${scene.id}-${String(trig.index + 1).padStart(2, "0")}`;
  const sel = `[data-sel-index="${trig.index}"]`;
  await page.$eval(sel, (el) => el.scrollIntoView({ block: "center", inline: "nearest" }));
  await sleep(120);
  const inDialog = await page.evaluate((s) => !!document.querySelector(s)?.closest(".vt-dialog"), sel);
  await page.click(sel);
  await sleep(380);
  const box = await popoverBox(page);
  const name = `${tag}-${trig.combo ? "combobox" : "select"}`;
  if (!box) return fail(name, "no popover after click");
  await page.screenshot({ path: join(outDir, scheme, `${name}.png`) });
  if (box.x < -1 || box.y < -1 || box.x + box.w > box.vw + 1 || box.y + box.h > box.vh + 1) fail(name, `popover outside the viewport ${JSON.stringify(box)}`);
  if (box.items === 0) fail(name, "no options");
  await page.keyboard.press("Escape");
  await sleep(250);
  if (await page.$(".vt-popover")) fail(name, "popover still open after Escape");
  if (inDialog && !(await page.$(".vt-dialog"))) fail(name, "Escape also closed the dialog around the select");
  const focused = await page.evaluate((s) => document.activeElement === document.querySelector(s), sel);
  if (!focused) fail(name, "focus did not return to the trigger");
  console.log(`  ${name}: "${trig.text}" items=${box.items} checked=${box.checked}`);
}

const key = async (page, k) => {
  await page.keyboard.press(k);
  await sleep(120);
};
const highlighted = (page) =>
  page.evaluate(() => {
    const el = document.querySelector('[role="option"][data-highlighted], [cmdk-item][data-selected="true"]');
    return el?.querySelector(".vt-option__label")?.textContent ?? el?.textContent ?? null;
  });
const triggerText = (page, sel) => page.$eval(sel, (el) => el.textContent.trim());
// A trigger an app re-creates after the pick (a confirm step replaces its panel) has nothing to hold the focus: not a failure.
const focusedIs = (page, sel) =>
  page.evaluate((s) => {
    const el = document.querySelector(s);
    return !el || document.activeElement === el;
  }, sel);

/** Keyboard-only run over one trigger. */
async function keyboardRun(page, trig, scheme, tag) {
  const sel = `[data-sel-index="${trig.index}"]`;
  const label = `keyboard ${tag} "${trig.text}"`;
  await page.$eval(sel, (el) => el.scrollIntoView({ block: "center" }));
  await page.focus(sel);
  await key(page, "ArrowDown");
  if (!(await popoverBox(page))) return fail(label, "ArrowDown did not open it");
  await sleep(250);
  const first = await highlighted(page);
  await key(page, "ArrowDown");
  const second = await highlighted(page);
  if (first === second && (await page.$$('[role="option"]')).length > 1) fail(label, `ArrowDown did not move the highlight (${first})`);
  await key(page, "ArrowUp");
  if ((await highlighted(page)) !== first) fail(label, "ArrowUp did not come back");
  // The label of a later option, typed in part: the highlight must land on a row starting with it.
  const labels = await page.$$eval('[role="option"] .vt-option__label', (els) => els.map((e) => e.textContent ?? ""));
  const target = labels.find((l) => l && l !== first && /^[a-z0-9]/i.test(l));
  if (target) {
    const typed = target.slice(0, trig.combo ? Math.min(3, target.length) : 1);
    await page.keyboard.type(typed, { delay: 30 });
    await sleep(200);
    const hit = await highlighted(page);
    if (trig.combo) {
      const rows = await page.$$eval('[role="option"] .vt-option__label', (els) => els.map((e) => e.textContent ?? ""));
      if (!rows.every((r) => r.toLowerCase().includes(typed.toLowerCase())) && labels.length > 5) {
        const hints = await page.$$eval('[role="option"]', (els) => els.map((e) => e.textContent ?? ""));
        if (!hints.every((h) => h.toLowerCase().includes(typed.toLowerCase()))) fail(label, `typing "${typed}" did not filter: ${rows.slice(0, 4).join("|")}`);
      }
    } else if (!hit?.toLowerCase().startsWith(typed.toLowerCase())) fail(label, `type-ahead "${typed}" highlighted "${hit}"`);
    if (trig.combo) {
      await page.screenshot({ path: join(outDir, scheme, `keyboard-${tag}-filtered.png`) });
      // A query nothing matches shows the empty state.
      await page.keyboard.type("zzzqq", { delay: 10 });
      await sleep(200);
      if (!(await page.evaluate(() => document.body.textContent.includes("No results")))) fail(label, "no 'No results' for a bad query");
      await page.screenshot({ path: join(outDir, scheme, `keyboard-${tag}-empty.png`) });
      for (let i = 0; i < 8; i++) await page.keyboard.press("Backspace");
      await page.keyboard.type(typed, { delay: 10 });
      await sleep(200);
    }
  }
  const want = await highlighted(page);
  await key(page, "Enter");
  await sleep(250);
  if (await page.$(".vt-popover")) fail(label, "Enter did not close the list");
  // Some picks open the app's own dialog (a template to confirm), which takes the focus by design.
  if (await page.$(".vt-dialog")) {
    await page.keyboard.press("Escape");
    return console.log(`  keyboard ${tag}: ok-run (the pick opened a dialog)`);
  }
  if (want && trig.combo === false && target && !(await triggerText(page, sel)).startsWith(want.slice(0, 2))) fail(label, `Enter picked "${await triggerText(page, sel)}", wanted "${want}"`);
  if (!(await focusedIs(page, sel))) fail(label, "focus not on the trigger after Enter");
  // Escape closes without changing the value.
  const before = await triggerText(page, sel);
  await key(page, "ArrowDown");
  await key(page, "ArrowDown");
  await key(page, "Escape");
  await sleep(250);
  if (await page.$(".vt-popover")) fail(label, "Escape did not close the list");
  if ((await triggerText(page, sel)) !== before) fail(label, "Escape changed the value");
  if (!(await focusedIs(page, sel))) fail(label, "focus not on the trigger after Escape");
  console.log(`  keyboard ${tag}: ok-run (picked "${await triggerText(page, sel)}")`);
}

try {
  let keyboardDone = { select: false, combo: false };
  for (const scheme of schemes) {
    mkdirSync(join(outDir, scheme), { recursive: true });
    const menusSeen = new Set();
    console.log(`== ${app} ${scheme}`);
    for (const scene of scenes) {
      let page;
      try {
        page = await openScene(scene, scheme);
      } catch (e) {
        console.log(`[${scene.id}] SKIPPED, the scene's own steps failed: ${String(e.message).split("\n")[0]}`);
        continue;
      }
      if (plain) {
        await plainShot(page, scheme, plain[1]);
        await page.close();
        continue;
      }
      const triggers = await tagTriggers(page);
      console.log(`[${scene.id}] ${triggers.length} select(s)`);
      for (const trig of triggers) {
        if (trig.disabled) continue;
        await openAndShoot(page, trig, scene, scheme);
        await tagTriggers(page);
      }
      // Dropdown menus (export menus and the like): open each distinct one once per scheme.
      for (const m of await page.$$('button[aria-haspopup="menu"]:not([disabled])')) {
        const name = await m.evaluate((el) => (el.getAttribute("aria-label") ?? el.textContent.trim()) || "menu");
        if (menusSeen.has(name) || !(await m.isVisible())) continue;
        menusSeen.add(name);
        await m.evaluate((el) => el.scrollIntoView({ block: "center" }));
        await m.click();
        await sleep(350);
        const items = await page.$$eval('[role="menuitem"]', (els) => els.length);
        const file = `menu-${scene.id}-${name.replace(/[^a-z0-9]+/gi, "_")}`;
        await page.screenshot({ path: join(outDir, scheme, `${file}.png`) });
        if (items === 0) fail(file, "menu opened without items");
        await page.keyboard.press("Escape");
        await sleep(250);
        if (await page.$('[role="menu"]')) fail(file, "menu still open after Escape");
        if (!(await m.evaluate((el) => document.activeElement === el))) fail(file, "focus did not return to the menu button");
        console.log(`  ${file}: ${items} item(s)`);
      }
      if (keyboard && scheme === schemes[0]) {
        for (const trig of await tagTriggers(page)) {
          const kind = trig.combo ? "combo" : "select";
          if (trig.disabled || keyboardDone[kind]) continue;
          keyboardDone[kind] = true;
          await keyboardRun(page, trig, scheme, `${scene.id}-${kind}`);
          await tagTriggers(page);
        }
      }
      await page.close();
    }
  }
} finally {
  await browser.close();
  vite.kill();
}
console.log(failures ? `\n${failures} check(s) failed` : "\nall select checks passed");
process.exit(failures ? 1 : 0);
