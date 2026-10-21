// App Store scenes for BLE Inspector. Mock data models a room full of BLE peripherals (a heart rate strap, a
// thermometer, a smart bulb and so on) with the standard GATT services the Rust side would discover.
//
// Mock command shapes follow apps/ble-inspector/src-tauri/src/main.rs and crates/mt-ble/src/types.rs
// (values are lowercase hex strings, UUIDs are the 36 character form, the ble-event payload is
// { type: "characteristic_notify", device_id, char_uuid, value }).

export const uiScale = 1.1;

const base = (short) => `0000${short}-0000-1000-8000-00805f9b34fb`;
const vendor = (n) => `6e40000${n}-b5a3-f393-e0a9-e50e24dcca9e`;

// [id, name, rssi, address]
const DEVICES = [
  ["D1", "Polar H10 7A3F2E1B", -52, "E4:6C:9A:21:7F:B0"],
  ["D2", "Govee H5075_4C21", -61, "A4:C1:38:4C:21:09"],
  ["D3", "Mi Smart Band 8", -66, "C8:0F:10:3B:92:D4"],
  ["D4", "Nordic_Beacon", -71, "F2:19:7B:04:AA:5E"],
  ["D5", "LEDnetWF 0A12", -74, "08:65:F0:11:0A:12"],
  ["D6", "Tile", -79, "D0:35:44:9E:61:C7"],
  ["D7", "ESP32 Sensor Node", -83, "7C:DF:A1:5B:E2:48"],
  ["D8", "Garmin Edge 540", -86, "FA:42:8D:73:10:6B"],
  ["D9", "(unknown)", -90, "5B:12:E9:D4:07:38"],
  ["D10", "JBL Flip 6", -92, "20:3A:4F:C6:81:DD"],
];

const hex = (s) => [...s].map((c) => c.charCodeAt(0).toString(16).padStart(2, "0")).join("");

const P = (...flags) => ({
  read: flags.includes("r"),
  write: flags.includes("w"),
  write_without_response: flags.includes("n"),
  notify: flags.includes("N"),
  indicate: flags.includes("i"),
});

// Services of device D1 (Polar H10), D5 (LED controller) and a generic fallback.
const GATT = {
  D1: [
    {
      uuid: base("180d"),
      is_primary: true,
      characteristics: [
        { uuid: base("2a37"), properties: P("N"), value: null },
        { uuid: base("2a38"), properties: P("r"), value: null },
        { uuid: base("2a39"), properties: P("w"), value: null },
      ],
    },
    {
      uuid: base("180f"),
      is_primary: true,
      characteristics: [{ uuid: base("2a19"), properties: P("r", "N"), value: null }],
    },
    {
      uuid: base("180a"),
      is_primary: true,
      characteristics: [
        { uuid: base("2a29"), properties: P("r"), value: null },
        { uuid: base("2a24"), properties: P("r"), value: null },
        { uuid: base("2a26"), properties: P("r"), value: null },
        { uuid: base("2a27"), properties: P("r"), value: null },
      ],
    },
    {
      uuid: vendor(1),
      is_primary: true,
      characteristics: [
        { uuid: vendor(2), properties: P("w", "n"), value: null },
        { uuid: vendor(3), properties: P("N"), value: null },
      ],
    },
  ],
};
const READ_VALUES = {
  [base("2a38")]: "01",
  [base("2a19")]: "5a",
  [base("2a29")]: hex("Polar Electro Oy"),
  [base("2a24")]: hex("H10"),
  [base("2a26")]: hex("5.0.0"),
  [base("2a27")]: hex("HW 4.2"),
};

function mockMain(D) {
  const T0 = Date.UTC(2026, 9, 21, 10, 14, 0);
  const realNow = Date.now.bind(Date);
  const r0 = realNow();
  window.__skew = 0;
  Date.now = () => T0 + (realNow() - r0) + window.__skew;
  // Headless screen == viewport, which the Toolbar reads as native fullscreen and drops the traffic-light inset.
  Object.defineProperty(window.screen, "width", { get: () => 2560 });
  Object.defineProperty(window.screen, "height", { get: () => 1600 });

  const cbs = new Map();
  const listeners = new Map();
  let nextCb = 1;
  let nextEv = 1;
  window.__emit = (event, payload) => {
    // React StrictMode (dev server) registers each listener twice; deliver to the newest only.
    const ls = listeners.get(event) ?? [];
    const l = ls[ls.length - 1];
    const fn = l && cbs.get(l.handler);
    if (fn) fn({ event, id: l.id, payload });
  };

  const state = { scanning: false, connected: new Set(), shown: 0 };
  window.__scanTo = (n) => { state.shown = n; };
  const list = () =>
    D.devices.slice(0, state.shown).map(([id, name, rssi, address]) => ({
      id, name, rssi, address, connected: state.connected.has(id),
    }));

  const handlers = {
    store_status: () => window.__store ?? { store: false, pro: true, reason: "build", price: null },
    "plugin:event|listen": ({ event, handler }) => {
      const id = nextEv++;
      if (!listeners.has(event)) listeners.set(event, []);
      listeners.get(event).push({ id, handler });
      return id;
    },
    "plugin:event|unlisten": ({ event, eventId }) => {
      listeners.set(event, (listeners.get(event) ?? []).filter((l) => l.id !== eventId));
      return null;
    },
    init_ble: () => null,
    start_scan: () => { state.scanning = true; return null; },
    stop_scan: () => { state.scanning = false; return null; },
    is_scanning: () => state.scanning,
    get_devices: () => list(),
    connect_device: ({ id }) => { state.connected.add(id); return null; },
    disconnect_device: ({ id }) => { state.connected.delete(id); return null; },
    discover_services: ({ id }) => D.gatt[id] ?? D.gatt.D1,
    read_characteristic: ({ charUuid }) => D.reads[charUuid] ?? "00",
    write_characteristic: () => null,
    subscribe_characteristic: () => null,
    unsubscribe_characteristic: () => null,
    open_url: () => null,
  };

  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(fn) {
      const id = nextCb++;
      cbs.set(id, fn);
      return id;
    },
    unregisterCallback(id) {
      cbs.delete(id);
    },
    convertFileSrc: (p) => p,
    async invoke(cmd, args) {
      const h = handlers[cmd];
      if (!h) {
        console.warn("unmocked command", cmd);
        throw "unmocked command " + cmd;
      }
      return h(args ?? {});
    },
  };
}

export const mock = `(${mockMain.toString()})(${JSON.stringify({ devices: DEVICES, gatt: GATT, reads: READ_VALUES })})`;

// ───────────────────────────── scene helpers ─────────────────────────────

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function clickText(page, selector, text, what = "startsWith") {
  const ok = await page.evaluate(
    (selector, text, what) => {
      const el = [...document.querySelectorAll(selector)].find((e) => {
        const t = e.textContent.trim();
        return what === "equals" ? t === text : t.startsWith(text);
      });
      if (!el) return false;
      el.click();
      return true;
    },
    selector,
    text,
    what,
  );
  if (!ok) throw new Error(`no ${selector} with text "${text}"`);
  await sleep(200);
}

async function scan(page, count) {
  await clickText(page, "button", "Start scan");
  await page.evaluate((n) => window.__scanTo(n), 4);
  await sleep(700);
  await page.evaluate((n) => window.__scanTo(n), count);
  await sleep(700);
}

async function connect(page, name) {
  await clickText(page, ".ble-device", name);
  await page.waitForSelector(".ble-svc", { timeout: 5000 });
  await sleep(500);
}

async function expand(page, shortUuid) {
  await clickText(page, ".ble-svc", `Service${shortUuid}`);
}

async function readAll(page) {
  const n = await page.evaluate(() => [...document.querySelectorAll(".ble-char button")].filter((b) => b.textContent.trim() === "Read").length);
  for (let i = 0; i < n; i++) {
    await page.evaluate((i) => {
      [...document.querySelectorAll(".ble-char button")].filter((b) => b.textContent.trim() === "Read")[i].click();
    }, i);
    await sleep(120);
  }
  await sleep(300);
}

async function typeInto(page, selector, text) {
  await page.click(selector);
  await page.type(selector, text, { delay: 20 });
}

async function feedHeartRate(page, bpms) {
  for (const b of bpms) {
    await page.evaluate((b) => {
      window.__skew += 1000;
      window.__emit("ble-event", {
        type: "characteristic_notify",
        device_id: "D1",
        char_uuid: "00002a37-0000-1000-8000-00805f9b34fb",
        value: "00" + b.toString(16).padStart(2, "0"),
      });
    }, b);
    await sleep(40);
  }
  await sleep(300);
}

// ───────────────────────────── scenes ─────────────────────────────

export const scenes = [
  {
    id: "scan",
    headline: "See every <em>Bluetooth</em> device nearby",
    sub: "Scan for BLE peripherals and compare signal strength, names and addresses at a glance.",
    setup: async (page) => {
      await scan(page, 10);
    },
  },
  {
    id: "browse",
    headline: "Browse the <em>GATT</em> tree in one click",
    sub: "Connect to a device and expand its services and characteristics, with read, write and notify properties shown.",
    settleMs: 800,
    setup: async (page) => {
      await scan(page, 10);
      await connect(page, "Polar H10");
      await expand(page, "0x180F");
      await expand(page, "0x180A");
    },
  },
  {
    id: "read",
    headline: "Read values and <em>keep a log</em>",
    sub: "Read any characteristic on demand. Every action lands in a timestamped communication log.",
    settleMs: 800,
    setup: async (page) => {
      await scan(page, 10);
      await connect(page, "Polar H10");
      await expand(page, "0x180D");
      await expand(page, "0x180F");
      await expand(page, "0x180A");
      await readAll(page);
    },
  },
  {
    id: "notify",
    headline: "Watch <em>notifications</em> live",
    sub: "Subscribe to a characteristic and see each update from the device as it arrives.",
    settleMs: 800,
    setup: async (page) => {
      await scan(page, 10);
      await connect(page, "Polar H10");
      await clickText(page, ".ble-char button", "Notify");
      await sleep(300);
      await feedHeartRate(page, [72, 73, 73, 74, 76, 79, 83, 88, 92, 95, 97, 96, 94, 91]);
    },
  },
  {
    id: "write",
    headline: "Write values to <em>test a device</em>",
    sub: "Send hex to a writable characteristic and confirm the result in the log.",
    settleMs: 800,
    setup: async (page) => {
      await scan(page, 10);
      await connect(page, "Polar H10");
      await expand(page, "0x180D");
      await expand(page, "0x0001");
      const sel = 'input[aria-label$="6e400002-b5a3-f393-e0a9-e50e24dcca9e"]';
      await typeInto(page, sel, "0a1b02ff");
      await clickText(page, ".ble-char button", "Write");
      await sleep(300);
      await typeInto(page, sel, "0100");
      await clickText(page, ".ble-char button", "Write");
      await sleep(300);
      await typeInto(page, sel, "ff00ff");
      await sleep(200);
    },
  },
  // Sixth scene: the in-app purchase dialog, rendered only as the App Review screenshot of the Pro product
  // (the first five scenes are the store screenshots).
  {
    id: "pro",
    headline: "Pro",
    sub: "Pro dialog",
    init: () => {
      window.__store = { store: true, pro: false, reason: "none", price: "$4.99" };
    },
    setup: async (page) => {
      await scan(page, 10);
      await connect(page, "Polar H10");
      await clickText(page, ".ble-char button", "Notify");
      await expand(page, "0x180F");
      await sleep(300);
      await page.evaluate(() => {
        const bs = [...document.querySelectorAll(".ble-char button")].filter((b) => b.textContent.trim().startsWith("Notify"));
        bs[bs.length - 1].click();
      });
      await page.waitForSelector(".vt-dialog", { timeout: 5000 });
      await sleep(400);
    },
  },
];
