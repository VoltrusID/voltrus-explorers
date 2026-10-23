// Drives the iPhone layout of Modbus Poll or BACnet Explorer in Chromium (iPhone 15: 393x852 @3x, touch, iOS user agent)
// with the mocked Tauri runtime the App Store scenes use, and saves one plain screenshot per screen, light and dark.
//
//   bun marketing/screenshots/render-phone.mjs <modbus-poll|bacnet-explorer> [outDir] [--scheme=light|dark|both] [--only=a,b]
//
// Default outDir is /tmp/<app>-phone. Safe areas are simulated through CDP Emulation.setSafeAreaInsetsOverride with the
// iPhone 15 portrait insets (status bar / Dynamic Island 59, home indicator 34), so `env(safe-area-inset-*)` is real here.
// Pass --no-insets to render without them. A translucent status-bar and home-indicator mock is painted over the page
// (outside the app's DOM) so a header drawn under the status bar is visible in the capture.
// Every screen is also audited: controls under 44px, horizontal overflow, content under the status bar or the home
// indicator, and inputs under 16px (iOS zooms the page on focus).
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import puppeteer from "puppeteer-core";
import { pickOption } from "./pick.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const args = process.argv.slice(2);
const flag = (name) => args.find((a) => a.startsWith(`--${name}=`))?.slice(name.length + 3);
const [app, outArg] = args.filter((a) => !a.startsWith("--"));
if (!["modbus-poll", "bacnet-explorer"].includes(app ?? "")) throw new Error("usage: render-phone.mjs <modbus-poll|bacnet-explorer> [outDir]");
const outDir = outArg ?? `/tmp/${app}-phone`;
const schemes = flag("scheme") === "light" ? ["light"] : flag("scheme") === "dark" ? ["dark"] : ["light", "dark"];
const only = flag("only")?.split(",");
const insets = !args.includes("--no-insets");
mkdirSync(outDir, { recursive: true });

const sceneModule = await import(pathToFileURL(join(here, app, "scenes.mjs")).href);
const { mock } = sceneModule;
const PORT = Number(process.env.SCREENSHOT_PORT ?? 1441);
const vite = spawn("bun", ["run", "dev", "--port", String(PORT), "--strictPort"], { cwd: join(root, "apps", app), stdio: ["ignore", "pipe", "inherit"] });
await new Promise((resolve, reject) => {
  vite.stdout.on("data", (d) => String(d).includes("ready in") && resolve());
  vite.on("exit", (c) => reject(new Error(`vite exited ${c}`)));
});

const IPHONE_UA = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const FONT_CSS = `:root { --font-sans: "Inter", system-ui, sans-serif !important; --font-mono: "JetBrains Mono", ui-monospace, monospace !important; } * { caret-color: transparent !important; } *::-webkit-scrollbar { display: none; } * { scrollbar-width: none; }`;
const INSET = { top: 59, bottom: 34, left: 0, right: 0 };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const browser = await puppeteer.launch({ executablePath: process.env.CHROMIUM ?? "/usr/bin/chromium", headless: true, args: ["--font-render-hinting=none", "--disable-lcd-text"] });

const results = [];

/** Opens a fresh page with the mock runtime, the scheme and the safe areas. `init` runs before the app. */
async function open(scheme, init = "") {
  const page = await browser.newPage();
  await page.setUserAgent(IPHONE_UA);
  await page.setViewport({ width: 393, height: 852, deviceScaleFactor: 3, isMobile: true, hasTouch: true });
  await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: scheme }]);
  if (insets) {
    const cdp = await page.createCDPSession();
    await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: INSET });
  }
  await page.evaluateOnNewDocument(mock);
  await page.evaluateOnNewDocument(`${init}\n${app === "modbus-poll" ? `localStorage.setItem("modbus-poll-theme", "${scheme}");` : ""}`);
  page.on("pageerror", (e) => console.error("page error:", e.message));
  page.on("console", (m) => m.type() === "error" && !String(m.text()).includes("Failed to load resource") && console.error("console error:", m.text()));
  await page.goto(`http://localhost:${PORT}/`, { waitUntil: "networkidle0" });
  await page.addStyleTag({ content: FONT_CSS });
  await sleep(400);
  return page;
}

/** Paints the status bar (clock, island) and the home indicator over the page, like the device does. */
async function chrome(page, scheme) {
  if (!insets) return;
  await page.evaluate((scheme, inset) => {
    document.getElementById("__sim")?.remove();
    const ink = scheme === "dark" ? "#fff" : "#000";
    const el = document.createElement("div");
    el.id = "__sim";
    el.style.cssText = "position:fixed;inset:0;pointer-events:none;z-index:2147483647;";
    el.innerHTML = `<div style="position:absolute;top:0;left:0;right:0;height:${inset.top}px;font:600 16px -apple-system,Inter,sans-serif;color:${ink};display:flex;justify-content:space-between;align-items:center;padding:6px 32px 0 36px"><span>9:41</span><span style="position:absolute;left:50%;top:11px;width:124px;height:36px;margin-left:-62px;background:#000;border-radius:20px"></span><span style="letter-spacing:2px">▂▄▆ ▮</span></div><div style="position:absolute;bottom:8px;left:50%;width:140px;height:5px;margin-left:-70px;border-radius:3px;background:${ink}"></div>`;
    document.body.appendChild(el);
  }, scheme, INSET);
}

async function audit(page, label) {
  const report = await page.evaluate((inset) => {
    const vw = window.innerWidth;
    const small = [];
    const tiny = [];
    for (const el of document.querySelectorAll("button, input, select, textarea, [role=tab], a")) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      if (el.closest("[hidden]") || el.closest("#__sim")) continue;
      if (el.type === "checkbox" || el.type === "radio") continue;
      const name = `${el.tagName.toLowerCase()} "${(el.getAttribute("aria-label") || el.textContent || el.placeholder || "").trim().slice(0, 22)}" ${Math.round(r.width)}x${Math.round(r.height)}`;
      if (r.height < 44 && !el.closest(".vt-segmented") && !el.closest(".vt-table") && !el.closest(".ob-row")) small.push(name);
      if (["INPUT", "SELECT", "TEXTAREA"].includes(el.tagName) && parseFloat(getComputedStyle(el).fontSize) < 16) tiny.push(name);
    }
    // Anything interactive or textual that starts inside the status-bar band.
    const under = [];
    for (const el of document.querySelectorAll("header *, nav button")) {
      const r = el.getBoundingClientRect();
      if (r.height && r.top < inset.top - 1 && r.bottom > 0 && el.children.length === 0 && (el.textContent || "").trim()) under.push((el.textContent || "").trim().slice(0, 20));
    }
    const nav = document.querySelector("nav");
    const navBottom = nav ? Math.round(window.innerHeight - nav.getBoundingClientRect().bottom) : null;
    return { small, tiny, under, overflowX: document.documentElement.scrollWidth > vw, navBottom };
  }, INSET);
  const bad = report.small.length + report.tiny.length + report.under.length + (report.overflowX ? 1 : 0);
  const line = `[audit ${label}] <44px: ${report.small.length}${report.small.length ? " -> " + report.small.slice(0, 5).join("; ") : ""} | <16px inputs: ${report.tiny.length}${report.tiny.length ? " -> " + report.tiny.slice(0, 3).join("; ") : ""} | under status bar: ${report.under.length}${report.under.length ? " -> " + report.under.join(",") : ""} | overflow-x: ${report.overflowX} | nav bottom gap: ${report.navBottom}`;
  results.push({ label, bad });
  console.log(line);
}

async function shot(page, scheme, name) {
  await sleep(350);
  await chrome(page, scheme);
  const file = join(outDir, `${name}-${scheme}.png`);
  await page.screenshot({ path: file });
  console.log(file);
  await audit(page, `${name}-${scheme}`);
}


/** Opens the SheetSelect whose trigger shows `text` or carries that aria-label. */
async function openTrigger(page, text) {
  const ok = await page.evaluate((text) => {
    const b = [...document.querySelectorAll("button[data-sheet-select]")].find((el) => el.textContent.trim().startsWith(text) || el.getAttribute("aria-label") === text);
    if (!b) return false;
    b.click();
    return true;
  }, text);
  if (!ok) {
    const seen = await page.evaluate(() => [...document.querySelectorAll("button[data-sheet-select]")].map((e) => e.textContent.trim() || e.getAttribute("aria-label")));
    throw new Error(`no sheet select "${text}"; saw ${seen.join(" | ")}`);
  }
  await page.waitForSelector(".vm-sheet-list", { timeout: 5000 });
  await sleep(450);
}

/** Swipes the open sheet down from its header with real touch events; fails the audit if it stays open. */
async function closeSheetBySwipe(page) {
  const box = await page.$eval(".vt-dialog__header", (el) => {
    const r = el.getBoundingClientRect();
    return { x: r.x + 60, y: r.y + r.height / 2 };
  });
  const cdp = await page.createCDPSession();
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: box.x, y: box.y }] });
  for (let dy = 20; dy <= 160; dy += 20) await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: box.x, y: box.y + dy }] });
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await sleep(500);
  if (await page.$(".vt-dialog")) console.error("swipe down did not dismiss the sheet");
  else console.log("swipe down dismissed the sheet");
}

async function closeSheetByTapOutside(page) {
  await page.mouse.click(196, 60);
  await sleep(500);
  if (await page.$(".vt-dialog")) console.error("tap outside did not dismiss the sheet");
  else console.log("tap outside dismissed the sheet");
}

const wants = (name) => !only || only.includes(name);

/** Scrolls the biggest scrollable element to `y` ("end" for the bottom). */
async function scrollMain(page, y) {
  await page.evaluate((y) => {
    const cands = [...document.querySelectorAll("*")].filter((e) => {
      const s = getComputedStyle(e);
      return (s.overflowY === "auto" || s.overflowY === "scroll") && e.scrollHeight > e.clientHeight + 20 && e.clientHeight > 300;
    });
    cands.sort((a, b) => b.clientHeight - a.clientHeight);
    const el = cands[0];
    if (el) el.scrollTop = y === "end" ? el.scrollHeight : y;
  }, y);
  await sleep(250);
}

async function clickSel(page, selector, text, { exact = false } = {}) {
  const ok = await page.evaluate((selector, text, exact) => {
    const el = [...document.querySelectorAll(selector)].find((e) => {
      if (e.offsetParent === null && getComputedStyle(e).position !== "fixed") return false;
      const t = e.textContent.trim();
      return exact ? t === text : t.startsWith(text);
    });
    if (!el) return false;
    el.click();
    return true;
  }, selector, text, exact);
  if (!ok) {
    const seen = await page.evaluate((selector) => [...document.querySelectorAll(selector)].map((e) => `${e.textContent.trim().slice(0, 20)}${e.offsetParent === null ? " (hidden)" : ""}`), selector);
    throw new Error(`no visible ${selector} with text ${text}; saw: ${seen.join(" | ")}`);
  }
  await sleep(250);
}

async function setStore(page, patch) {
  await page.evaluate(async (patchSrc) => {
    const { useStore } = await import("/src/store.ts");
    useStore.setState(new Function("return " + patchSrc)());
  }, patch);
  await sleep(200);
}

async function waitPolls(page, n) {
  await page.waitForFunction((count) => {
    const chip = document.querySelector("[data-polls]");
    if (chip) return Number(chip.getAttribute("data-polls")) >= count;
    const m = /([\d,]+) polls/.exec(document.querySelector("footer")?.textContent ?? "");
    return m && Number(m[1].replace(/,/g, "")) >= count;
  }, { timeout: 60000 }, n);
}

// ── Modbus Poll ───────────────────────────────────────────────────────────────────────────────────
async function modbus(scheme) {
  const { iosScenes } = sceneModule;
  const init = iosScenes[0].init;
  let page = await open(scheme, init);
  if (wants("connect")) {
    await shot(page, scheme, "01-connect");
    await scrollMain(page, 700);
    await shot(page, scheme, "02-connect-scrolled");
    await scrollMain(page, "end");
    await shot(page, scheme, "03-connect-end");
    await scrollMain(page, 0);
  }
  if (wants("tabs-empty")) {
    for (const [tab, name] of [["data", "04-data-empty"], ["blocks", "05-blocks"], ["log", "06-log-empty"], ["traffic", "07-traffic-empty"], ["trend", "08-trend-empty"]]) {
      await setStore(page, `({ mobileTab: "${tab}" })`);
      await shot(page, scheme, name);
    }
    await setStore(page, `({ mobileTab: "connect" })`);
  }
  if (wants("more")) {
    const more = await page.$$eval("nav [role=tab], nav button", (els) => els.some((e) => e.textContent.trim().startsWith("More")));
    if (more) {
      await clickSel(page, "nav [role=tab], nav button", "More");
      await shot(page, scheme, "09-more-sheet");
      await page.keyboard.press("Escape");
      await sleep(300);
      await page.evaluate(() => document.querySelector(".vm-sheet__scrim, .vm-scrim")?.click());
      await sleep(300);
    }
  }
  if (wants("live")) {
    await pickOption(page, "New profile", "MCC-1 incomer");
    await clickSel(page, "button", "Connect", { exact: true });
    await waitPolls(page, 25);
    await sleep(1500);
    await shot(page, scheme, "10-connect-connected");
    await scrollMain(page, 0);
    await setStore(page, `({ mobileTab: "data" })`);
    await sleep(800);
    await shot(page, scheme, "11-data");
    await setStore(page, `({ mobileTab: "log" })`);
    await sleep(600);
    await shot(page, scheme, "12-log");
    await setStore(page, `({ mobileTab: "traffic" })`);
    await sleep(900);
    await shot(page, scheme, "13-traffic");
    await page.evaluate(() => {
      const rows = [...document.querySelectorAll('[title="Click to expand frame details"]')];
      rows.find((r, k) => k >= 3 && r.textContent.includes("RX"))?.click();
    });
    await sleep(400);
    await shot(page, scheme, "14-traffic-expanded");
    await page.evaluate(async () => {
      const { useStore } = await import("/src/store.ts");
      const colors = ["#3b82f6", "#ef4444", "#22c55e"];
      const now = Date.now();
      const m = new Map();
      ["PM5560_3005", "PM5560_3007", "PM5560_3009"].forEach((name, i) => {
        const points = [];
        for (let s = 150; s >= 1; s--) points.push({ t: now - s * 1000, v: window.__mock.meter(-s)[["i1", "i2", "i3"][i]] });
        m.set(name, { name, color: colors[i], points });
      });
      useStore.setState({ trendSeries: m, trendWindow: 300 });
    });
    await setStore(page, `({ mobileTab: "trend" })`);
    await sleep(2500);
    await shot(page, scheme, "15-trend");
    await setStore(page, `({ mobileTab: "blocks" })`);
    await sleep(500);
    await shot(page, scheme, "16-blocks");
    await setStore(page, `({ mobileTab: "data" })`);
    await sleep(500);
    await page.evaluate(() => {
      const row = [...document.querySelectorAll(".vm-row[role=button]")].find((r) => !r.textContent.includes("ON") && !r.textContent.includes("OFF"));
      row?.click();
    });
    await sleep(500);
    await shot(page, scheme, "17-write-sheet");
  }
  await page.close();

  if (wants("pickers")) {
    // Every list opens as a bottom sheet. Seed twelve profiles so the profile sheet is searchable.
    const many = JSON.stringify(
      Array.from({ length: 12 }, (_, i) => ({
        name: i < 3 ? `MCC-${i + 1} incomer` : `Panel ${i + 1}`,
        transport: "tcp",
        tcp: { host: `10.0.0.${i + 1}`, port: 502, unitId: 1, pollInterval: 1000 },
        rtu: { port: "", baudRate: 9600, dataBits: 8, stopBits: 1, parity: "none", unitId: 1 },
        blocks: [{ name: "Registers", address: 0, quantity: 10, type: "uint16", area: "holding" }],
      })),
    );
    page = await open(scheme, `${init}\nlocalStorage.setItem("modbus-poll-conn-profiles", ${JSON.stringify(many)});`);
    await openTrigger(page, "New profile");
    await shot(page, scheme, "40-sheet-profiles-search");
    await page.type('.vt-dialog input[type="search"]', "mcc");
    await sleep(300);
    await shot(page, scheme, "41-sheet-profiles-filtered");
    const rows = await page.$$eval('.vm-sheet-list [role="option"]', (els) => els.length);
    if (rows !== 3) console.error(`profile search: expected 3 rows for "mcc", got ${rows}`);
    await closeSheetBySwipe(page);
    await openTrigger(page, "New profile");
    await closeSheetByTapOutside(page);
    await scrollMain(page, "end");
    await openTrigger(page, "AB CD");
    await shot(page, scheme, "42-sheet-word-order");
    await page.keyboard.press("Escape");
    await sleep(400);
    await openTrigger(page, "Decimal");
    await shot(page, scheme, "43-sheet-value-format");
    await page.keyboard.press("Escape");
    await sleep(400);
    await scrollMain(page, 0);
    await openTrigger(page, "Connection");
    await shot(page, scheme, "44-sheet-connections");
    await page.keyboard.press("Escape");
    await sleep(400);
    await setStore(page, `({ mobileTab: "blocks" })`);
    await sleep(300);
    await scrollMain(page, "end");
    await openTrigger(page, "holding");
    await shot(page, scheme, "45-sheet-area");
    await page.keyboard.press("Escape");
    await sleep(400);
    await openTrigger(page, "uint16");
    await shot(page, scheme, "46-sheet-type");
    await page.close();

    // Block picker on the Data tab, with a connected device.
    page = await open(scheme, init);
    await pickOption(page, "New profile", "MCC-1 incomer");
    await clickSel(page, "button", "Connect", { exact: true });
    await waitPolls(page, 10);
    await setStore(page, `({ mobileTab: "data" })`);
    await sleep(500);
    await openTrigger(page, "Read block");
    await shot(page, scheme, "47-sheet-blocks");
    await page.close();

    // Write value: the type picker opens a sheet over the sheet.
    page = await open(scheme, init);
    await pickOption(page, "New profile", "MCC-1 incomer");
    await clickSel(page, "button", "Connect", { exact: true });
    await waitPolls(page, 10);
    await setStore(page, `({ mobileTab: "data" })`);
    await sleep(500);
    await page.evaluate(() => [...document.querySelectorAll(".vm-row[role=button]")].find((r) => !r.textContent.includes("ON") && !r.textContent.includes("OFF"))?.click());
    await sleep(500);
    await openTrigger(page, "Float32");
    await shot(page, scheme, "48-sheet-over-sheet");
    await page.close();
  }

  if (wants("slave")) {
    page = await open(scheme, init);
    await setStore(page, `({ mode: "slave", slavePollMs: 500 })`);
    await sleep(300);
    await shot(page, scheme, "20-slave-connect");
    await clickSel(page, "button", "Start server", { exact: true });
    await sleep(500);
    await shot(page, scheme, "21-slave-running");
    await setStore(page, `({ mobileTab: "template" })`);
    await sleep(300);
    await shot(page, scheme, "22-slave-template");
    await pickOption(page, "Select device…", "pm5560", "Schneider Electric PM5560");
    await sleep(500);
    await clickSel(page, "button", "Simulate", { exact: true });
    await sleep(2500);
    await shot(page, scheme, "23-slave-simulating");
    await setStore(page, `({ mobileTab: "data" })`);
    await sleep(1200);
    await shot(page, scheme, "24-slave-data");
    await page.close();
  }

  if (wants("sheets")) {
    page = await open(scheme, iosScenes.find((s) => s.id === "connect-profiles").init);
    const hasAccount = await page.$('button[aria-label^="Voltrus Connect"]');
    if (hasAccount) {
      await page.click('button[aria-label^="Voltrus Connect"]');
      await sleep(900);
      await shot(page, scheme, "30-account-sheet");
      await page.keyboard.press("Escape");
      await sleep(400);
    }
    await page.close();

    // Pro upsell: an App Store build that has not unlocked Pro.
    page = await open(
      scheme,
      `${init}\n(() => { const inv = window.__TAURI_INTERNALS__.invoke; window.__TAURI_INTERNALS__.invoke = (cmd, a) => (cmd === "store_status" ? Promise.resolve({ store: true, pro: false, reason: "free", price: null }) : inv(cmd, a)); })();`,
    );
    await clickSel(page, "[role=radio]", "Slave");
    await sleep(600);
    await shot(page, scheme, "31-pro-sheet");
    await page.close();

    page = await open(scheme, init);
    const info = await page.$('button[aria-label="About"]');
    if (info) {
      await info.click();
      await sleep(500);
      await shot(page, scheme, "32-about");
    }
    await page.close();
  }
}

// ── BACnet Explorer ───────────────────────────────────────────────────────────────────────────────
async function bacnet(scheme) {
  const init = `window.__platform = "ios";`;
  let page = await open(scheme, init);
  const tab = (t) => clickSel(page, "nav [role=tab]", t);
  if (wants("devices")) {
    await shot(page, scheme, "01-devices");
    await scrollMain(page, "end");
    await shot(page, scheme, "02-devices-end");
    await scrollMain(page, 0);
  }
  if (wants("tabs")) {
    for (const [t, name] of [["Objects", "03-objects-empty"], ["Trend", "04-trend-empty"], ["Alarms", "05-alarms-empty"], ["Log", "06-log-empty"]]) {
      await tab(t);
      await shot(page, scheme, name);
    }
    await tab("Devices");
  }
  if (wants("live")) {
    await clickSel(page, ".vm-body button", "Discover devices");
    await page.waitForSelector(".vt-dialog .vt-btn--primary", { timeout: 5000 });
    await shot(page, scheme, "10-discovery");
    await clickSel(page, ".vt-dialog button", "Connect", { exact: true });
    await page.waitForFunction(() => document.querySelector('nav [role=tab][aria-selected="true"]')?.textContent.includes("Objects"), { timeout: 5000 });
    await sleep(500);
    await shot(page, scheme, "11-objects-connected");
    await clickSel(page, "button", "Browse objects");
    await page.waitForSelector(".ob-group");
    await clickSel(page, ".ob-group", "Analog Input");
    await shot(page, scheme, "12-objects-expanded");
    for (const name of ["SA-TEMP", "RA-TEMP"]) {
      await page.evaluate((name) => {
        const row = [...document.querySelectorAll(".ob-row")].find((r) => r.querySelector("span")?.textContent === name);
        row.querySelector('button[aria-label="Add to trend"]').click();
      }, name);
      await sleep(150);
    }
    await clickSel(page, ".ob-row", "SA-TEMP");
    await sleep(400);
    await shot(page, scheme, "13-properties");
    await clickSel(page, "button", "Objects", { exact: true });
    await tab("Trend");
    await page.evaluate(() => document.querySelectorAll('button[title^="Data source: COV"]').forEach((b) => b.click()));
    await page.evaluate(async () => {
      for (let i = 0; i < 40; i++) {
        window.__skew += 3000;
        for (const [inst, v] of [[1, 13.2 + Math.sin(i / 5)], [2, 23.1 + Math.cos(i / 7) * 0.4]]) {
          window.__emit("cov-notification", { subscriber_process_identifier: 1, initiating_device_identifier: 2001, monitored_object_type: "analogInput", monitored_object_instance: inst, time_remaining: 3500, values: [{ property_name: "presentValue", value: { Real: v }, raw_value: v.toFixed(2) }], timestamp: Date.now() });
        }
        await new Promise((r) => setTimeout(r, 2));
      }
    });
    await shot(page, scheme, "14-trend");
    await tab("Alarms");
    await clickSel(page, "button", "Refresh");
    await page.waitForSelector(".vm-body .mono");
    await clickSel(page, "button", "Events");
    await sleep(300);
    await shot(page, scheme, "15-alarms");
    await tab("Log");
    await shot(page, scheme, "16-log");
    await tab("Devices");
    await shot(page, scheme, "17-devices-connected");
  }
  await page.close();

  if (wants("sim")) {
    // The simulator: starting it opens the control panel as a full-screen sheet (iOS has a single window).
    page = await open(scheme, init);
    await pickOption(page, "Select template…", "ahu", "AHU-1 (Air Handling Unit)");
    await sleep(300);
    await shot(page, scheme, "20-simulator-setup");
    await clickSel(page, ".vm-body button", "Start simulation");
    await sleep(400);
    await page.evaluate(() => window.__simTick(40));
    await shot(page, scheme, "21-simulator-sheet");
    await clickSel(page, ".bx-sheet button", "Back");
    await scrollMain(page, "end");
    await shot(page, scheme, "22-simulator-devices");
    await page.close();
    // Discovery without the multicast entitlement.
    page = await open(scheme, `${init} window.__discoverFail = "Who-Is broadcast could not be sent: No route to host (os error 65)";`);
    await clickSel(page, ".vm-body button", "Discover devices");
    await page.waitForSelector(".vt-callout");
    await shot(page, scheme, "23-discovery-no-multicast");
    await page.close();
  }

  if (wants("sheets")) {
    page = await open(scheme, sceneModule.iosScenes.find((s) => s.id === "connect-profiles").init);
    const hasAccount = await page.$('button[aria-label^="Voltrus Connect"]');
    if (hasAccount) {
      await page.click('button[aria-label^="Voltrus Connect"]');
      await sleep(900);
      await shot(page, scheme, "30-account-sheet");
    }
    await page.close();
    page = await open(scheme, init);
    const info = await page.$('button[aria-label="About"]');
    if (info) {
      await info.click();
      await sleep(500);
      await shot(page, scheme, "32-about");
    }
    await page.close();
  }
}

try {
  for (const scheme of schemes) await (app === "modbus-poll" ? modbus(scheme) : bacnet(scheme));
  const bad = results.filter((r) => r.bad > 0);
  console.log(`\n${results.length} screens audited, ${bad.length} with findings${bad.length ? ": " + bad.map((r) => r.label).join(", ") : ""}`);
} finally {
  await browser.close();
  vite.kill();
}
