// Renders App Store screenshots (2880x1800) for one app from its scene file.
//
//   bun marketing/screenshots/render.mjs <app> [--device=iphone|ipad] [--bare] [sceneId...]
//
// Starts the app's Vite dev server, opens it in Chromium with a mocked Tauri runtime
// (marketing/screenshots/<app>/scenes.mjs exports `mock` and `scenes`), drives each scene to the state
// it shows, captures the app at 1180x740 CSS px (device scale 2), and composes it into frame.html with
// the scene's headline. Output: marketing/screenshots/<app>/out/<NN>-<id>.png
//
// --bare skips frame.html and writes the raw app capture (1180x740 CSS at 2x on the Mac, the device viewport at 3x
// for iphone/ipad) as out/bare-<NN>-<id>.png (out/bare-ios-<device>-<NN>-<id>.png), for website use. Bare renders
// emulate prefers-color-scheme: dark so apps that follow the system theme match the dark site.
//
// A scene with `crop: "<selector>"` also writes out/bare-<NN>-<id>-crop.png around that element.
// Scenes flagged `review` (the Voltrus Connect website scenes in connect-docs.mjs) render only when named.
// A scene file may export `uiScale` (> 1 renders the UI larger inside the same window size).
// SCREENSHOT_PORT picks the dev-server port (default 1420) so two apps can render at once.
// Requirements: Chromium at $CHROMIUM (default /usr/bin/chromium) and the Inter font installed.
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import puppeteer from "puppeteer-core";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const args = process.argv.slice(2);
const deviceArg = args.find((a) => a.startsWith("--device="))?.slice(9);
const bare = args.includes("--bare");
const [app, ...only] = args.filter((a) => !a.startsWith("--device=") && a !== "--bare");
if (!app) throw new Error("usage: bun marketing/screenshots/render.mjs <app> [--device=iphone|ipad] [--bare] [sceneId...]");

const sceneModule = await import(pathToFileURL(join(here, app, "scenes.mjs")).href);
// --device=iphone|ipad renders the scene file's `iosScenes` at App Store size (iPhone 6.9" 1320x2868, iPad 13" 2064x2752).
const DEVICES = {
  iphone: { css: [390, 844], out: [1320, 2868], ua: "iPhone; CPU iPhone OS 18_0 like Mac OS X", cls: "phone" },
  ipad: { css: [834, 1112], out: [2064, 2752], ua: "iPad; CPU OS 18_0 like Mac OS X", cls: "tablet" },
};
const device = deviceArg ? DEVICES[deviceArg] : null;
if (deviceArg && !device) throw new Error(`unknown device ${deviceArg}`);
const { mock, uiScale = 1 } = sceneModule;
const scenes = device ? sceneModule.iosScenes.filter((s) => !s.devices || s.devices.includes(deviceArg)) : sceneModule.scenes;
const outDir = join(here, app, "out");
mkdirSync(outDir, { recursive: true });

const PORT = Number(process.env.SCREENSHOT_PORT ?? 1420);
const vite = spawn("bun", ["run", "dev", "--port", String(PORT), "--strictPort"], {
  cwd: join(root, "apps", app),
  stdio: ["ignore", "pipe", "inherit"],
});
await new Promise((resolve, reject) => {
  vite.stdout.on("data", (d) => String(d).includes("ready in") && resolve());
  vite.on("exit", (c) => reject(new Error(`vite exited ${c}`)));
});

const browser = await puppeteer.launch({
  executablePath: process.env.CHROMIUM ?? "/usr/bin/chromium",
  headless: true,
  args: ["--font-render-hinting=none", "--disable-lcd-text"],
});

// Mac UA so the shared Toolbar reserves the traffic-light inset like the real macOS window.
const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
// SF Pro/Mono are not available here; Inter and JetBrains Mono are the closest metric matches.
const FONT_CSS = `:root { --font-sans: "Inter", system-ui, sans-serif !important; --font-mono: "JetBrains Mono", ui-monospace, monospace !important; }
  * { caret-color: transparent !important; }`;

try {
  const list = only.length ? scenes.filter((s) => only.includes(s.id)) : scenes.filter((s) => !s.review);
  for (const scene of list) {
    const page = await browser.newPage();
    if (device) {
      await page.setUserAgent(`Mozilla/5.0 (${device.ua}) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1`);
      await page.setViewport({ width: device.css[0], height: device.css[1], deviceScaleFactor: 3, isMobile: true, hasTouch: true });
    } else {
      await page.setUserAgent(MAC_UA);
      await page.setViewport({ width: Math.round(1180 / uiScale), height: Math.round(740 / uiScale), deviceScaleFactor: 2 * uiScale });
    }
    if (bare) await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await page.evaluateOnNewDocument(mock);
    if (scene.init) await page.evaluateOnNewDocument(scene.init);
    page.on("pageerror", (e) => console.error(`[${scene.id}] page error:`, e.message));
    await page.goto(`http://localhost:${PORT}/${scene.hash ?? ""}`, { waitUntil: "networkidle0" });
    await page.addStyleTag({ content: FONT_CSS });
    await scene.setup(page, deviceArg);
    await new Promise((r) => setTimeout(r, scene.settleMs ?? 600));
    const shot = await page.screenshot({ type: "png" });
    // `crop` (a selector, bare renders only) also writes out/bare-<NN>-<id>-crop.png: that element plus a 32px margin.
    let crop = null;
    if (bare && scene.crop) {
      const box = await page.evaluate((sel) => {
        const r = document.querySelector(sel)?.getBoundingClientRect();
        return r && { x: r.x, y: r.y, width: r.width, height: r.height };
      }, scene.crop);
      if (box) {
        const pad = 32;
        const x = Math.max(0, box.x - pad);
        const y = Math.max(0, box.y - pad);
        const vp = page.viewport();
        crop = await page.screenshot({
          type: "png",
          clip: { x, y, width: Math.min(vp.width - x, box.width + 2 * pad), height: Math.min(vp.height - y, box.height + 2 * pad) },
        });
      }
    }
    await page.close();

    const prefix = `${bare ? "bare-" : ""}${device ? `ios-${deviceArg}-` : ""}${String(scenes.indexOf(scene) + 1).padStart(2, "0")}-${scene.id}.png`;
    if (bare) {
      const file = join(outDir, prefix);
      writeFileSync(file, shot);
      console.log(file);
      if (crop) {
        const cropFile = file.replace(/\.png$/, "-crop.png");
        writeFileSync(cropFile, crop);
        console.log(cropFile);
      }
      continue;
    }

    const frame = await browser.newPage();
    const [fw, fh] = device ? [device.out[0] / 2, device.out[1] / 2] : [1440, 900];
    await frame.setViewport({ width: fw, height: fh, deviceScaleFactor: 2 });
    await frame.goto(pathToFileURL(join(here, device ? "frame-ios.html" : "frame.html")).href);
    if (device) await frame.evaluate((w, h, cls) => {
      document.documentElement.style.setProperty("--w", `${w}px`);
      document.documentElement.style.setProperty("--h", `${h}px`);
      document.body.className = cls;
    }, fw, fh, device.cls);
    await frame.evaluate(
      (headline, sub, src) => {
        document.getElementById("headline").innerHTML = headline;
        document.getElementById("sub").textContent = sub;
        document.getElementById("shot").src = src;
      },
      scene.headline,
      scene.sub,
      `data:image/png;base64,${Buffer.from(shot).toString("base64")}`,
    );
    await frame.waitForFunction(() => document.getElementById("shot").complete);
    const file = join(outDir, prefix);
    writeFileSync(file, await frame.screenshot({ type: "png" }));
    await frame.close();
    console.log(file);
  }
} finally {
  await browser.close();
  vite.kill();
}
