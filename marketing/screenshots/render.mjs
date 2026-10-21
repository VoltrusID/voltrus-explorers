// Renders App Store screenshots (2880x1800) for one app from its scene file.
//
//   bun marketing/screenshots/render.mjs <app> [sceneId...]
//
// Starts the app's Vite dev server, opens it in Chromium with a mocked Tauri runtime
// (marketing/screenshots/<app>/scenes.mjs exports `mock` and `scenes`), drives each scene to the state
// it shows, captures the app at 1180x740 CSS px (device scale 2), and composes it into frame.html with
// the scene's headline. Output: marketing/screenshots/<app>/out/<NN>-<id>.png
//
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
const [app, ...only] = process.argv.slice(2);
if (!app) throw new Error("usage: bun marketing/screenshots/render.mjs <app> [sceneId...]");

const { mock, scenes, uiScale = 1 } = await import(pathToFileURL(join(here, app, "scenes.mjs")).href);
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
  const list = only.length ? scenes.filter((s) => only.includes(s.id)) : scenes;
  for (const scene of list) {
    const page = await browser.newPage();
    await page.setUserAgent(MAC_UA);
    await page.setViewport({ width: Math.round(1180 / uiScale), height: Math.round(740 / uiScale), deviceScaleFactor: 2 * uiScale });
    await page.evaluateOnNewDocument(mock);
    if (scene.init) await page.evaluateOnNewDocument(scene.init);
    page.on("pageerror", (e) => console.error(`[${scene.id}] page error:`, e.message));
    await page.goto(`http://localhost:${PORT}/${scene.hash ?? ""}`, { waitUntil: "networkidle0" });
    await page.addStyleTag({ content: FONT_CSS });
    await scene.setup(page);
    await new Promise((r) => setTimeout(r, scene.settleMs ?? 600));
    const shot = await page.screenshot({ type: "png" });
    await page.close();

    const frame = await browser.newPage();
    await frame.setViewport({ width: 1440, height: 900, deviceScaleFactor: 2 });
    await frame.goto(pathToFileURL(join(here, "frame.html")).href);
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
    const file = join(outDir, `${String(scenes.indexOf(scene) + 1).padStart(2, "0")}-${scene.id}.png`);
    writeFileSync(file, await frame.screenshot({ type: "png" }));
    await frame.close();
    console.log(file);
  }
} finally {
  await browser.close();
  vite.kill();
}
