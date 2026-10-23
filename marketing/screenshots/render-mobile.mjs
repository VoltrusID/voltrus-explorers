// Drives BACnet Explorer's phone and iPad layouts in Chromium with touch emulation and the mocked Tauri runtime
// (the same `mock` the App Store scenes use), and saves plain screenshots (no marketing frame).
//
//   bun marketing/screenshots/render-mobile.mjs [outDir]      # default /tmp/bacnet-mobile
//
// Viewports: iPhone 15 (393x852 @3x), iPad portrait (820x1180 @2) and iPad landscape (1180x820 @2).
// Touch emulation makes `(pointer: coarse)` match, so the 44pt controls and always-visible write buttons apply.
// env(safe-area-inset-*) is 0 in Chromium; the notch and home indicator are not simulated.
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import puppeteer from "puppeteer-core";
import { pickOption } from "./pick.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const outDir = process.argv[2] ?? "/tmp/bacnet-mobile";
mkdirSync(outDir, { recursive: true });
const { mock } = await import(pathToFileURL(join(here, "bacnet-explorer/scenes.mjs")).href);

const PORT = Number(process.env.SCREENSHOT_PORT ?? 1431);
const vite = spawn("bun", ["run", "dev", "--port", String(PORT), "--strictPort"], {
  cwd: join(root, "apps/bacnet-explorer"),
  stdio: ["ignore", "pipe", "inherit"],
});
await new Promise((resolve, reject) => {
  vite.stdout.on("data", (d) => String(d).includes("ready in") && resolve());
  vite.on("exit", (c) => reject(new Error(`vite exited ${c}`)));
});

const IPHONE_UA =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const IPAD_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const FONT_CSS = `:root { --font-sans: "Inter", system-ui, sans-serif !important; --font-mono: "JetBrains Mono", ui-monospace, monospace !important; } * { caret-color: transparent !important; }`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const browser = await puppeteer.launch({
  executablePath: process.env.CHROMIUM ?? "/usr/bin/chromium",
  headless: true,
  args: ["--font-render-hinting=none", "--disable-lcd-text"],
});

async function open(viewport, ua, init = "") {
  const page = await browser.newPage();
  await page.setUserAgent(ua);
  await page.setViewport({ ...viewport, isMobile: true, hasTouch: true });
  await page.evaluateOnNewDocument(mock);
  await page.evaluateOnNewDocument(`window.__platform = "ios"; ${init}`);
  page.on("pageerror", (e) => console.error("page error:", e.message));
  page.on("console", (m) => m.type() === "error" && console.error("console error:", m.text()));
  await page.goto(`http://localhost:${PORT}/`, { waitUntil: "networkidle0" });
  await page.addStyleTag({ content: FONT_CSS });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await sleep(300);
  return page;
}

async function clickText(page, selector, text, exact = false) {
  const ok = await page.evaluate(
    (selector, text, exact) => {
      const el = [...document.querySelectorAll(selector)].find((e) => {
        if (e.offsetParent === null && getComputedStyle(e).position !== "fixed") return false; // hidden page
        const t = e.textContent.trim();
        return exact ? t === text : t.startsWith(text);
      });
      if (!el) return false;
      el.click();
      return true;
    },
    selector,
    text,
    exact,
  );
  if (!ok) throw new Error(`no visible ${selector} with text ${text}`);
  await sleep(180);
}

async function typeInto(page, handleIndex, selector, value) {
  const inputs = await page.$$(selector);
  await inputs[handleIndex].click();
  await page.keyboard.down("Control");
  await page.keyboard.press("a");
  await page.keyboard.up("Control");
  await page.keyboard.type(String(value));
}

async function shot(page, name) {
  await sleep(350);
  const file = join(outDir, `${name}.png`);
  await page.screenshot({ path: file });
  console.log(file);
}

// Layout assertions: every control that is actually visible must be at least 36pt tall and nothing may overflow the viewport.
async function audit(page, label) {
  const report = await page.evaluate(() => {
    const vw = window.innerWidth;
    const small = [];
    for (const el of document.querySelectorAll("button, input, select, [role=tab]")) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      if (el.closest("[hidden]")) continue;
      if (r.height < 36 && !el.closest(".vt-segmented")) small.push(`${el.tagName.toLowerCase()} "${(el.getAttribute("aria-label") || el.textContent || "").trim().slice(0, 24)}" ${Math.round(r.width)}x${Math.round(r.height)}`);
    }
    return { small, overflowX: document.documentElement.scrollWidth > vw };
  });
  console.log(`[audit ${label}] controls under 36px: ${report.small.length}${report.small.length ? " -> " + report.small.slice(0, 6).join("; ") : ""}; horizontal overflow: ${report.overflowX}`);
}

try {
  // ── iPhone ──
  const phone = { width: 393, height: 852, deviceScaleFactor: 3 };
  let page = await open(phone, IPHONE_UA);
  await shot(page, "phone-1-devices");
  await audit(page, "phone devices");
  await clickText(page, ".vm-body button", "Discover devices");
  await page.waitForSelector(".vt-dialog .vt-btn--primary", { timeout: 5000 });
  await shot(page, "phone-2-discovery");
  await clickText(page, ".vt-dialog button", "Connect", true);
  await page.waitForFunction(() => document.querySelector('nav [role="tab"][aria-selected="true"]')?.textContent.includes("Objects"), { timeout: 5000 });
  await clickText(page, "button", "Browse objects");
  await page.waitForSelector(".ob-group");
  await shot(page, "phone-3-objects");
  await clickText(page, ".ob-group", "Analog Input");
  await shot(page, "phone-4-objects-expanded");
  await audit(page, "phone objects");
  // add trend series from the list, then open a property page
  for (const name of ["SA-TEMP", "RA-TEMP"]) {
    await page.evaluate((name) => {
      const row = [...document.querySelectorAll(".ob-row")].find((r) => r.querySelector("span")?.textContent === name);
      row.querySelector('button[aria-label="Add to trend"]').click();
    }, name);
    await sleep(150);
  }
  await clickText(page, ".ob-row", "SA-TEMP");
  await sleep(300);
  await shot(page, "phone-5-properties");
  await audit(page, "phone properties");
  await clickText(page, "button", "Objects", true); // back
  await clickText(page, "nav [role=tab]", "Trend");
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
  await shot(page, "phone-6-trend");
  await audit(page, "phone trend");
  await clickText(page, "nav [role=tab]", "Alarms");
  await clickText(page, "button", "Refresh");
  await page.waitForSelector(".vm-body .mono");
  await clickText(page, "button", "Events");
  await sleep(300);
  await shot(page, "phone-7-alarms");
  await audit(page, "phone alarms");
  await clickText(page, "nav [role=tab]", "Log");
  await shot(page, "phone-8-log");
  await page.close();

  // iOS discovery without the multicast entitlement: the Who-Is send fails
  page = await open(phone, IPHONE_UA, `window.__discoverFail = "Who-Is broadcast could not be sent: No route to host (os error 65)";`);
  await clickText(page, ".vm-body button", "Discover devices");
  await page.waitForSelector(".vt-callout");
  await shot(page, "phone-9-discovery-no-multicast");
  await page.close();

  // simulator control panel sheet
  page = await open(phone, IPHONE_UA);
  await pickOption(page, "Select template…", "AHU-1 (Air Handling Unit)");
  await sleep(150);
  await clickText(page, ".vm-body button", "Start simulation");
  await sleep(300);
  await page.evaluate(() => window.__simTick(40));
  await shot(page, "phone-10-simulator-sheet");
  await clickText(page, ".bx-sheet button", "Back");
  await page.evaluate(() => { const a = document.querySelector(".vm-body"); a.scrollTop = a.scrollHeight; });
  await shot(page, "phone-11-simulator-devices");
  await page.close();

  // ── iPad landscape and portrait ──
  for (const [label, vp] of [["landscape", { width: 1180, height: 820, deviceScaleFactor: 2 }], ["portrait", { width: 820, height: 1180, deviceScaleFactor: 2 }]]) {
    page = await open(vp, IPAD_UA);
    await shot(page, `ipad-${label}-1-devices`);
    await typeInto(page, 0, "aside input", "10.20.4.21");
    await typeInto(page, 2, "aside input", "2001");
    await clickText(page, "aside button", "Connect", true);
    await page.waitForFunction(() => [...document.querySelectorAll("aside button")].some((b) => b.textContent.trim().startsWith("Alarms")));
    await clickText(page, "button", "Browse objects");
    await page.waitForSelector(".ob-group");
    await clickText(page, ".ob-group", "Analog Input");
    await clickText(page, ".ob-row", "SA-TEMP");
    await sleep(300);
    await shot(page, `ipad-${label}-2-browse`);
    await audit(page, `ipad ${label}`);
    await page.close();
  }
} finally {
  await browser.close();
  vite.kill();
}
