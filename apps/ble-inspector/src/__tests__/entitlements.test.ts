import { describe, expect, it } from "vitest";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const appsDir = join(__dirname, "..", "..", "..");

// Every app runs in the App Sandbox. Without `network.client` the sandboxed WKWebView cannot load
// the bundled frontend and the window stays blank (BLE Inspector shipped that way).
describe("app sandbox entitlements", () => {
  const apps = readdirSync(appsDir).filter((d) => existsSync(join(appsDir, d, "src-tauri", "entitlements.plist")));

  it("finds every app", () => {
    expect(apps).toContain("ble-inspector");
    expect(apps.length).toBeGreaterThanOrEqual(10);
  });

  for (const app of apps) {
    it(`${app} allows outgoing network access so the webview can load`, () => {
      const plist = readFileSync(join(appsDir, app, "src-tauri", "entitlements.plist"), "utf8");
      expect(plist).toMatch(/<key>com\.apple\.security\.network\.client<\/key>\s*<true\/>/);
    });
  }
});
