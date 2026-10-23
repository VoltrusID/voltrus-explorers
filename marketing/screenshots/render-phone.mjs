// Drives the iPhone layout of Modbus Poll, BACnet Explorer, S7 Explorer or OPC UA Explorer in Chromium (iPhone 15: 393x852 @3x, touch, iOS user agent)
// with the mocked Tauri runtime the App Store scenes use, and saves one plain screenshot per screen, light and dark.
//
//   bun marketing/screenshots/render-phone.mjs <modbus-poll|bacnet-explorer|s7-explorer|opcua-explorer> [outDir] [--scheme=light|dark|both] [--only=a,b]
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
if (!["modbus-poll", "bacnet-explorer", "s7-explorer", "opcua-explorer"].includes(app ?? "")) throw new Error("usage: render-phone.mjs <modbus-poll|bacnet-explorer|s7-explorer|opcua-explorer> [outDir]");
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

/** Taps the row titled `label` of the open sheet. */
async function pickRow(page, label) {
  const ok = await page.evaluate((label) => {
    const row = [...document.querySelectorAll('.vm-sheet-list [role="option"]')].find((el) => el.querySelector(".vm-row__title")?.textContent === label);
    row?.click();
    return !!row;
  }, label);
  if (!ok) throw new Error(`no sheet row "${label}"`);
  await sleep(450);
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
    await pickOption(page, "Select device…", "Schneider Electric PM5560");
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
    await pickOption(page, "Select template…", "AHU-1 (Air Handling Unit)");
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

// ── S7 Explorer ───────────────────────────────────────────────────────────────────────────────────
async function s7(scheme) {
  const { seedDevices, seedPhoneDevices } = sceneModule;
  const store = (status) => `window.__storeStatus = ${JSON.stringify(status)};`;
  const seed = `(${seedDevices})(); (${seedPhoneDevices})();`;
  const init = `${seed} ${store({ store: true, pro: true, reason: "purchase", price: null })}`;
  const free = `${seed} ${store({ store: true, pro: false, reason: "none", price: null })}`;
  const tab = (page, t) => clickSel(page, "nav [role=tab]", t);
  const press = (page, t, sel = ".vm-body button") => clickSel(page, sel, t, { exact: true });
  const type = async (page, label, value) => {
    const ok = await page.evaluate((label) => {
      const f = [...document.querySelectorAll(".vt-dialog .vt-field, .vm-body .vt-field")].filter((e) => e.offsetParent !== null).reverse().find((e) => e.querySelector("label")?.textContent.trim() === label);
      const input = f?.querySelector("input");
      if (!input) return false;
      input.focus();
      input.select();
      return true;
    }, label);
    if (!ok) throw new Error(`no input labelled ${label}`);
    await page.keyboard.type(String(value));
    await sleep(150);
  };
  const connect = async (page) => {
    await tab(page, "Connect");
    await press(page, "Connect");
    await waitPolls(page, 3);
  };
  const sheetRow = (page, title) => clickSel(page, ".vt-dialog .vm-row", title);
  const closeSheet = async (page) => {
    await page.click('.vt-dialog:last-of-type button[aria-label="Close"]').catch(() => {});
    await sleep(350);
  };

  let page = await open(scheme, init);
  if (wants("connect")) {
    await shot(page, scheme, "01-connect");
    await scrollMain(page, "end");
    await shot(page, scheme, "02-connect-end");
    await scrollMain(page, 0);
    await press(page, "Connect", ".vm-body form button");
    await page.waitForSelector('[data-connected="true"]', { timeout: 8000 });
    await sleep(500);
    await shot(page, scheme, "03-connect-connected");
  }
  if (wants("watch")) {
    if (!wants("connect")) await connect(page);
    await tab(page, "Watch");
    await sleep(800);
    await shot(page, scheme, "10-watch-live");
    await scrollMain(page, "end");
    await shot(page, scheme, "11-watch-end");
    await scrollMain(page, 0);
  }
  if (wants("blocks")) {
    if (!wants("connect") && !wants("watch")) await connect(page);
    await tab(page, "Blocks");
    await sleep(700);
    await shot(page, scheme, "20-blocks-types");
    await clickSel(page, ".vm-row", "DBData blocks");
    await shot(page, scheme, "21-blocks-db");
    await clickSel(page, ".vm-row", "DBDB 10");
    await sleep(700);
    await shot(page, scheme, "22-block-sheet");
    await press(page, "Next 16", ".vt-dialog button");
    await sleep(600);
    await press(page, "Save to file", ".vt-dialog button");
    await sleep(700);
    await shot(page, scheme, "23-block-saved");
    await press(page, "Delete from PLC", ".vt-dialog button");
    await sleep(500);
    await shot(page, scheme, "24-block-delete-confirm");
    await press(page, "Cancel", ".vt-dialog button");
    await closeSheet(page);
    await press(page, "Blocks", ".vm-body button");
    await sleep(300);
    await shot(page, scheme, "25-blocks-back");
  }
  if (wants("trend")) {
    if (!wants("connect") && !wants("watch") && !wants("blocks")) await connect(page);
    await tab(page, "Trend");
    await shot(page, scheme, "30-trend-empty");
    await tab(page, "Watch");
    for (const name of ["Pump speed rpm", "Pump current A", "Motor temp C"]) {
      await page.click(`button[aria-label="Add ${name} to trend"]`);
      await sleep(120);
    }
    await sleep(7000);
    await tab(page, "Trend");
    await sleep(600);
    await shot(page, scheme, "31-trend");
    await tab(page, "Watch");
  }
  if (wants("sheets")) {
    if (!wants("connect") && !wants("watch") && !wants("blocks") && !wants("trend")) await connect(page);
    await tab(page, "Watch");
    await clickSel(page, ".vm-row", "Pump speed rpm");
    await sleep(500);
    await shot(page, scheme, "40-write-real");
    await press(page, "Cancel", ".vt-dialog button");
    await clickSel(page, ".vm-row", "Pump running");
    await sleep(500);
    await shot(page, scheme, "41-write-bool");
    await press(page, "Write FALSE", ".vt-dialog button");
    await sleep(500);
    await clickSel(page, ".vm-row", "Cycle count");
    await sleep(300);
    await press(page, "Cancel", ".vt-dialog button");
    await press(page, "Tags", ".vm-body button");
    await sleep(500);
    await shot(page, scheme, "42-tags-sheet");
    await press(page, "Add tag", ".vt-dialog button");
    await sleep(500);
    await shot(page, scheme, "43-tag-new");
    await type(page, "Name", "Mixer speed");
    await type(page, "Address", "DB20.DBD0");
    await shot(page, scheme, "44-tag-filled");
    await press(page, "Cancel", ".vt-dialog button");
    await clickSel(page, ".vt-dialog .vm-row", "Tank level %");
    await sleep(400);
    await shot(page, scheme, "45-tag-edit");
    await press(page, "Cancel", ".vt-dialog button");
    await press(page, "Import CSV", ".vt-dialog button").catch(async () => clickSel(page, ".vt-dialog button", "Import CSV"));
    await sleep(900);
    await shot(page, scheme, "46-tags-import");
    await press(page, "Cancel", ".vt-dialog button");
    await closeSheet(page);
    await tab(page, "Connect");
    await scrollMain(page, "end");
    await press(page, "Remove Filling line", ".vm-body button");
    await sleep(450);
    await shot(page, scheme, "47-remove-confirm");
    await press(page, "Cancel", ".vt-dialog button");
    await page.click('button[aria-label="About"]');
    await sleep(500);
    await shot(page, scheme, "48-about");
    await closeSheet(page);
  }
  if (wants("pickers")) {
    await page.close();
    page = await open(scheme, init);
    await openTrigger(page, "S7-1200 / S7-1500");
    await shot(page, scheme, "50-picker-cpu");
    await closeSheetBySwipe(page);
    await openTrigger(page, "1 s");
    await shot(page, scheme, "51-picker-poll");
    await closeSheetByTapOutside(page);
    await openTrigger(page, "S7-1200 / S7-1500").catch(() => {});
  }
  if (wants("more")) {
    await page.close();
    page = await open(scheme, init);
    await connect(page);
    await tab(page, "More");
    await sleep(500);
    await shot(page, scheme, "60-more-sheet");
    await sheetRow(page, "Diagnostics");
    await sleep(1200);
    await shot(page, scheme, "61-diagnostics");
    await scrollMain(page, "end");
    await shot(page, scheme, "62-diagnostics-end");
    await tab(page, "More");
    await sheetRow(page, "Simulator");
    await sleep(500);
    await shot(page, scheme, "63-sim-setup");
    await pickOption(page, "Choose", "S7-300 Process Control (DB1)");
    await shot(page, scheme, "64-sim-template");
    await press(page, "Start server");
    await sleep(1500);
    await shot(page, scheme, "65-sim-running");
    await press(page, "Load its tags");
    await sleep(500);
    await shot(page, scheme, "66-watch-sim-tags");
    await tab(page, "More");
    await sheetRow(page, "Log");
    await sleep(400);
    await shot(page, scheme, "67-log");
    await page.close();
    // The App Store build with Pro locked: the upsell on a Pro tab, the live tag limit on Watch.
    page = await open(scheme, free);
    await tab(page, "Blocks");
    await sleep(600);
    await shot(page, scheme, "70-pro-upsell");
    await page.click('.vt-dialog button[aria-label="Close"]');
    await connect(page);
    await tab(page, "Watch");
    await sleep(600);
    await shot(page, scheme, "71-watch-free");
    await page.close();
  }
  await page.close().catch(() => {});
}

// ── OPC UA Explorer ───────────────────────────────────────────────────────────────────────────────
async function opcua(scheme) {
  let page = await open(scheme, "");
  const tab = (t) => clickSel(page, "nav [role=tab]", t);
  const moreTo = async (label) => {
    await tab("More");
    await page.waitForSelector(".vt-dialog .vm-row", { timeout: 5000 });
    await sleep(350);
    await clickSel(page, ".vt-dialog .vm-row", label);
    await sleep(500);
  };
  const closeSheet = async () => {
    await page.keyboard.press("Escape");
    await sleep(450);
  };
  const type = async (sel, value) => {
    const el = await page.$(sel);
    await el.click({ clickCount: 3 });
    await page.keyboard.press("Backspace");
    await el.type(value);
  };
  // Pages stay mounted when hidden, so a trigger is looked up among the visible ones.
  const pickVisible = async (current, label) => {
    const opened = await page.evaluate((current) => {
      const b = [...document.querySelectorAll("button[data-sheet-select]")].find((el) => el.offsetParent !== null && el.textContent.trim() === current);
      b?.click();
      return !!b;
    }, current);
    if (!opened) throw new Error(`no visible select showing "${current}"`);
    await page.waitForSelector(".vm-sheet-list", { timeout: 5000 });
    await sleep(400);
    await pickRow(page, label);
  };
  const trendOn = async (names) => {
    for (const name of names) {
      await page.evaluate((name) => document.querySelector(`button[aria-label="Add ${name} to trend"]`).click(), name);
      await sleep(300);
    }
  };

  if (wants("connect")) {
    await shot(page, scheme, "01-connect");
    await scrollMain(page, "end");
    await shot(page, scheme, "02-connect-end");
    await scrollMain(page, 0);
  }
  if (wants("tabs")) {
    for (const [t, name] of [["Browse", "03-browse-empty"], ["Watch", "04-watch-empty"], ["Trend", "05-trend-empty"]]) {
      await tab(t);
      await shot(page, scheme, name);
    }
    await tab("More");
    await page.waitForSelector(".vt-dialog .vm-row", { timeout: 5000 });
    await sleep(450);
    await shot(page, scheme, "06-more-sheet");
    await closeSheet();
    await moreTo("Server");
    await shot(page, scheme, "07-server");
    await moreTo("Namespaces");
    await shot(page, scheme, "08-namespaces-empty");
    await tab("Connect");
  }
  if (wants("pickers")) {
    await openTrigger(page, "None");
    await shot(page, scheme, "10-policy-sheet");
    await closeSheetBySwipe(page);
    await openTrigger(page, "None");
    await pickRow(page, "Basic256Sha256");
    await openTrigger(page, "Sign and encrypt");
    await shot(page, scheme, "11-mode-sheet");
    await closeSheetByTapOutside(page);
    await pickVisible("Anonymous", "User name and password");
    await sleep(200);
    await shot(page, scheme, "12-connect-secure");
    await clickSel(page, ".vm-body .vm-row", "Find servers");
    await page.waitForSelector(".vt-dialog .vm-row", { timeout: 5000 });
    await sleep(500);
    await shot(page, scheme, "13-discovery-endpoints");
    await clickSel(page, ".vt-dialog button", "Find servers", { exact: true });
    await page.waitForFunction(() => [...document.querySelectorAll(".vt-dialog .vm-row__title")].some((e) => e.textContent === "Plant Gateway"), { timeout: 5000 });
    await sleep(300);
    await shot(page, scheme, "14-discovery-servers");
    await clickSel(page, ".vt-dialog .vm-row", "Plant Gateway");
    await sleep(600);
    await clickSel(page, ".vt-dialog .vm-row", "Basic256Sha256");
    await sleep(500);
    await scrollMain(page, 0);
    await shot(page, scheme, "15-connect-filled");
    await scrollMain(page, "end");
    await clickSel(page, ".vm-body .vm-row", "Certificates");
    await page.waitForSelector(".oc-cert", { timeout: 5000 });
    await sleep(400);
    await shot(page, scheme, "16-certificates");
    await page.evaluate(() => document.querySelector(".vt-dialog__body")?.scrollTo(0, 9999));
    await shot(page, scheme, "17-certificates-end");
    await clickSel(page, ".vt-dialog button", "Trust this certificate");
    await sleep(500);
    await closeSheet();
    await scrollMain(page, 0);
  }
  if (wants("live")) {
    await page.evaluate(() => (document.querySelector('input[type="url"]').scrollIntoView(), null));
    const inputs = await page.$$(".vm-body input:not([type=url])");
    if (inputs[0]) await inputs[0].type("operator");
    if (inputs[1]) await inputs[1].type("plantfloor");
    await clickSel(page, "button", "Connect", { exact: true });
    await page.waitForFunction(() => document.querySelector('nav [role=tab][aria-selected="true"]')?.textContent.includes("Watch"), { timeout: 8000 });
    await sleep(900);
    await page.evaluate(() => window.__advance(10));
    await shot(page, scheme, "20-watch");
    await scrollMain(page, "end");
    await shot(page, scheme, "21-watch-end");
    await scrollMain(page, 0);
    await clickSel(page, ".oc-watch", "Pump1Speed");
    await page.waitForSelector(".vt-dialog", { timeout: 5000 });
    await sleep(400);
    await shot(page, scheme, "22-write-sheet");
    await closeSheet();
    await trendOn(["BoilerTemp", "SteamPressure", "Pump1Speed", "TankLevel"]);
    await shot(page, scheme, "23-watch-trending");
    await page.click('button[aria-label="Export CSV"]');
    await sleep(500);
    await shot(page, scheme, "24-export-sheet");
    await closeSheet();
    await page.click('button[aria-label="Subscriptions"]');
    await page.waitForSelector(".vt-dialog", { timeout: 5000 });
    await sleep(600);
    for (const n of ["ns=2;s=Boiler.Temperature", "ns=2;s=Pump1.MotorCurrent", "ns=2;s=Tank1.Level"]) {
      await type('.vt-dialog input[aria-label="Node ID to monitor"]', n);
      await page.keyboard.press("Enter");
      await sleep(120);
    }
    await shot(page, scheme, "25-subscriptions-setup");
    await page.evaluate(() => document.querySelector(".vt-dialog__body")?.scrollTo(0, 9999));
    await shot(page, scheme, "26-subscriptions-setup-end");
    await clickSel(page, ".vt-dialog button", "Start subscription");
    await sleep(500);
    await page.evaluate(() => window.__advance(12));
    await page.evaluate(() => document.querySelector(".vt-dialog__body")?.scrollTo(0, 0));
    await shot(page, scheme, "27-subscriptions-live");
    await clickSel(page, ".vt-dialog button", "Stop subscription");
    await closeSheet();

    await tab("Browse");
    await page.waitForSelector(".oc-crumb", { timeout: 5000 });
    await sleep(500);
    await shot(page, scheme, "30-browse-root");
    await clickSel(page, ".vm-row", "Plant");
    await sleep(500);
    await clickSel(page, ".vm-row", "Pump1");
    await sleep(500);
    await shot(page, scheme, "31-browse-pump");
    await clickSel(page, ".vm-row", "Speed");
    await page.waitForSelector(".vt-dialog", { timeout: 5000 });
    await sleep(600);
    await shot(page, scheme, "32-node-sheet");
    await page.evaluate(() => document.querySelector(".vt-dialog__body")?.scrollTo(0, 9999));
    await shot(page, scheme, "33-node-sheet-end");
    await clickSel(page, ".vt-dialog button", "References");
    await sleep(300);
    await page.evaluate(() => document.querySelector(".vt-dialog__body")?.scrollTo(0, 9999));
    await shot(page, scheme, "34-node-references");
    await closeSheet();
    await clickSel(page, ".oc-crumb", "Objects");
    await sleep(400);

    await tab("Trend");
    await page.evaluate(() => window.__advance(60));
    await sleep(500);
    await shot(page, scheme, "40-trend");
    await page.evaluate(() => [...document.querySelectorAll(".oc-scroll")].find((e) => e.offsetParent !== null)?.scrollTo(0, 9999));
    await shot(page, scheme, "41-trend-end");

    await moreTo("Namespaces");
    await page.waitForSelector(".oc-kv", { timeout: 5000 });
    await shot(page, scheme, "42-namespaces");
    await moreTo("Log");
    await shot(page, scheme, "43-log");
    await tab("Connect");
    await shot(page, scheme, "44-connect-connected");
  }
  await page.close();

  if (wants("server")) {
    page = await open(scheme, "");
    await moreTo("Server");
    await page.evaluate(() => document.querySelector("section")?.scrollIntoView());
    await pickVisible("None", "Process Plant (Reactor R-101)");
    await shot(page, scheme, "50-server-template");
    await clickSel(page, ".vm-body button", "Start simulation");
    await sleep(700);
    await page.evaluate(() => window.__advance(20));
    await scrollMain(page, 0);
    await shot(page, scheme, "51-server-running");
    await scrollMain(page, "end");
    await shot(page, scheme, "52-server-nodes");
    await page.close();
  }

  if (wants("about")) {
    page = await open(scheme, "");
    await page.click('button[aria-label="About"]');
    await sleep(500);
    await shot(page, scheme, "60-about");
    await page.close();
  }
}

try {
  for (const scheme of schemes) await (app === "modbus-poll" ? modbus(scheme) : app === "s7-explorer" ? s7(scheme) : app === "opcua-explorer" ? opcua(scheme) : bacnet(scheme));
  const bad = results.filter((r) => r.bad > 0);
  console.log(`\n${results.length} screens audited, ${bad.length} with findings${bad.length ? ": " + bad.map((r) => r.label).join(", ") : ""}`);
} finally {
  await browser.close();
  vite.kill();
}
