// App Store scenes for S7 Explorer. Rendered by marketing/screenshots/render.mjs.
//
// The mock stands in for the Rust side of the app. Command names, arguments and return shapes follow
// apps/s7-explorer/src-tauri/src/lib.rs and the frontend hooks: read_tags -> (number | boolean | string)[]
// in tag order, get_cpu_info / get_plc_mode / get_diag_buffer, list_blocks -> { block_type, block_number }[],
// sim_start -> { port, point_count }. The PLC is a filling line controller with a pump data block (DB10).

function installMock() {
  const T0 = Date.now();
  const callbacks = new Map();
  let nextCallback = 1;
  const listeners = [];
  let nextListener = 1;

  function emit(event, payload) {
    // StrictMode registers each listener twice in the browser; deliver to the newest one only.
    const l = listeners.filter((x) => x.event === event).pop();
    const cb = l && callbacks.get(l.handler);
    if (cb) cb({ event, id: l.id, payload });
  }

  const rnd = (k, t) => {
    const r = Math.sin(t * 12.9898 + k * 78.233) * 43758.5453;
    return r - Math.floor(r) - 0.5;
  };

  /** Value of an address at time t (seconds since page load). */
  const MODEL = {
    "DB10.DBD0": (t) => 1450 + 14 * Math.sin(t / 2.3) + 4 * rnd(1, t),
    "DB10.DBD4": (t) => 12.4 + 0.9 * Math.sin(t / 1.9 + 1) + 0.15 * rnd(2, t),
    "DB10.DBD8": (t) => 67.2 + 1.6 * Math.sin(t / 7) + 0.1 * rnd(3, t),
    "DB10.DBD12": (t) => 72.5 + 3.2 * Math.sin(t / 5.5 + 2) + 0.1 * rnd(4, t),
    "DB10.DBD16": (t) => 4.21 + 0.12 * Math.sin(t / 2.7) + 0.015 * rnd(5, t),
    "DB10.DBD20": (t) => 38.4 + 2.1 * Math.sin(t / 3.3 + 0.5) + 0.2 * rnd(6, t),
    "DB10.DBX24.0": () => true,
    "DB10.DBX24.1": () => false,
    "DB10.DBX24.2": (t) => Math.floor(t / 9) % 3 !== 0,
    "DB10.DBW26": (t) => 1247 + Math.floor(t / 4),
    "DB10.DBD28": (t) => 482913 + Math.floor(t * 3),
    MW50: () => 1450,
    "DB1.DBD0": (t) => 185.4 + 1.2 * Math.sin(t / 3) + 0.3 * rnd(21, t),
    "DB1.DBD4": (t) => 4.21 + 0.08 * Math.sin(t / 2.5) + 0.02 * rnd(22, t),
    "DB1.DBD8": (t) => 72.5 + 1.5 * Math.sin(t / 4) + 0.3 * rnd(23, t),
    "DB1.DBD12": (t) => 12.4 + 0.3 * Math.sin(t / 2.2) + 0.05 * rnd(24, t),
    "DB1.DBD16": (t) => 7.12 + 0.05 * Math.sin(t / 2) + 0.01 * rnd(25, t),
    "DB1.DBX20.0": () => true,
    "DB1.DBX20.1": () => true,
    "DB1.DBX20.2": () => false,
    "DB1.DBW24": (t) => 1247 + Math.floor(t / 6),
  };

  const f2 = (v, d) => Number(v.toFixed(d));
  const DECIMALS = { "DB1.DBD0": 1, "DB1.DBD4": 2, "DB1.DBD8": 1, "DB1.DBD12": 1, "DB1.DBD16": 2, "DB10.DBD0": 1, "DB10.DBD4": 2, "DB10.DBD8": 1, "DB10.DBD12": 1, "DB10.DBD16": 2, "DB10.DBD20": 1 };

  const state = { connected: false, simRunning: false };

  const commands = {
    // `window.__storeStatus` lets the phone harness render the App Store build (no outside links, Pro locked or unlocked).
    store_status: () => window.__storeStatus ?? { store: false, pro: true, reason: "build", price: null },
    store_purchase: () => ({ store: true, pro: true, reason: "purchase", price: null }),
    store_restore: () => ({ store: true, pro: true, reason: "purchase", price: null }),
    "plugin:dialog|open": () => "/private/var/mobile/Documents/line-tags.csv",
    "plugin:dialog|save": ({ options }) => `/private/var/mobile/Documents/${options?.defaultPath ?? "export"}`,
    "plugin:fs|read_text_file": () =>
      Array.from(new TextEncoder().encode("Name,Address,Type\nMixer speed,DB20.DBD0,Real\nMixer run,DB20.DBX4.0,Bool\nBatch count,DB20.DBW6,Word\nHopper level,DB20.DBD8,Real\n")),
    platform: () => "macos",
    open_url: () => null,
    connect: () => {
      state.connected = true;
      return null;
    },
    disconnect: () => {
      state.connected = false;
      return null;
    },
    read_tags: async ({ tags }) => {
      if (!state.connected) throw "Not connected";
      await new Promise((r) => setTimeout(r, 3 + Math.random() * 2));
      const t = (Date.now() - T0) / 1000;
      return tags.map((tag) => {
        // Raw bytes of a data block (DB10.DBB4): a fixed pattern.
        const raw = /^DB(\d+)\.DBB(\d+)$/.exec(tag.address);
        if (raw) return (Number(raw[1]) * 31 + Number(raw[2]) * 17) % 256;
        const f = MODEL[tag.address];
        if (!f) return 0;
        const v = f(t);
        return typeof v === "number" && DECIMALS[tag.address] !== undefined ? f2(v, DECIMALS[tag.address]) : v;
      });
    },
    write_tag: () => null,
    get_cpu_info: () => ({
      module_type: "CPU 1515-2 PN",
      serial_number: "S C-J2F314502021",
      as_name: "FillingLine_PLC",
      module_name: "CPU 1515-2 PN",
      firmware_version: "V2.9.4",
    }),
    get_plc_mode: () => ({ mode: "RUN" }),
    get_diag_buffer: () => ({
      entry_count: 8,
      entries: [
        { event_number: 16, timestamp: "2026-10-21 08:41:12.318", event_text: "Communication initiated: new connection on port 102" },
        { event_number: 15, timestamp: "2026-10-21 07:02:44.091", event_text: "Diagnostic data block DB10 accessed" },
        { event_number: 14, timestamp: "2026-10-21 06:30:00.004", event_text: "Cycle time monitoring: scan cycle 2.4 ms, maximum 6.1 ms" },
        { event_number: 13, timestamp: "2026-10-20 22:15:37.552", event_text: "Module Pump drive (slot 3): maintenance demanded cleared" },
        { event_number: 12, timestamp: "2026-10-20 22:14:58.120", event_text: "Module Pump drive (slot 3): maintenance demanded" },
        { event_number: 11, timestamp: "2026-10-20 18:00:03.870", event_text: "Mode transition: STARTUP to RUN" },
        { event_number: 10, timestamp: "2026-10-20 18:00:01.236", event_text: "Mode transition: STOP to STARTUP (warm restart)" },
        { event_number: 9, timestamp: "2026-10-20 17:59:40.015", event_text: "Power on, memory card recognized" },
      ],
    }),
    list_blocks: () => [
      { block_type: "OB", block_number: 1 },
      { block_type: "OB", block_number: 35 },
      { block_type: "OB", block_number: 100 },
      { block_type: "FC", block_number: 1 },
      { block_type: "FC", block_number: 12 },
      { block_type: "FC", block_number: 20 },
      { block_type: "FB", block_number: 1 },
      { block_type: "FB", block_number: 10 },
      { block_type: "DB", block_number: 1 },
      { block_type: "DB", block_number: 10 },
      { block_type: "DB", block_number: 11 },
      { block_type: "DB", block_number: 20 },
    ],
    upload_block: () => [0, 1, 2],
    delete_plc_block: () => null,
    sim_start: ({ points, port }) => {
      state.simRunning = true;
      setTimeout(() => {
        emit("sim-log", { level: "info", message: `S7 server listening on 0.0.0.0:${port}` });
        emit("sim-log", { level: "info", message: `Loaded ${points.length} points into DB1` });
      }, 50);
      return { port, point_count: points.length };
    },
    sim_stop: () => {
      state.simRunning = false;
      return null;
    },
    sim_set_value: () => null,
    sim_set_bool: () => null,
    "plugin:event|listen": ({ event, handler }) => {
      const id = nextListener++;
      listeners.push({ id, event, handler });
      return id;
    },
    "plugin:event|unlisten": ({ eventId }) => {
      const k = listeners.findIndex((l) => l.id === eventId);
      if (k >= 0) listeners.splice(k, 1);
      return null;
    },
  };

  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(fn) {
      const id = nextCallback++;
      callbacks.set(id, fn);
      return id;
    },
    unregisterCallback(id) { callbacks.delete(id); },
    convertFileSrc: (p) => p,
    async invoke(cmd, args) {
      const handler = commands[cmd];
      if (handler) return handler(args || {});
      if (cmd.startsWith("plugin:")) return null;
      throw new Error(`mock: unknown command ${cmd}`);
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  Object.defineProperty(screen, "width", { get: () => 2560 });
  Object.defineProperty(screen, "height", { get: () => 1600 });
  const scrollIntoView = Element.prototype.scrollIntoView;
  Element.prototype.scrollIntoView = function (arg) {
    return scrollIntoView.call(this, arg && typeof arg === "object" ? { ...arg, behavior: "auto" } : arg);
  };
}

export const mock = `(${installMock.toString()})();`;

/** Seeds the persisted device list before the app loads. */
function initDevice() {
  const id = (n) => `00000000-0000-4000-8000-00000000000${n}`;
  const tag = (n, name, address, data_type) => ({ id: id(n), name, address, data_type });
  localStorage.setItem(
    "s7-explorer-devices",
    JSON.stringify([
      {
        id: "10000000-0000-4000-8000-000000000001",
        label: "Filling line",
        host: "192.168.10.20",
        port: 102,
        rack: 0,
        slot: 1,
        tags: [
          tag(1, "Pump speed rpm", "DB10.DBD0", "Real"),
          tag(2, "Pump current A", "DB10.DBD4", "Real"),
          tag(3, "Motor temp C", "DB10.DBD8", "Real"),
          tag(4, "Tank level %", "DB10.DBD12", "Real"),
          tag(5, "Line pressure bar", "DB10.DBD16", "Real"),
          tag(6, "Flow m3/h", "DB10.DBD20", "Real"),
          tag(7, "Pump running", "DB10.DBX24.0", "Bool"),
          tag(8, "Pump fault", "DB10.DBX24.1", "Bool"),
          tag(9, "Inlet valve open", "DB10.DBX24.2", "Bool"),
          tag(10, "Cycle count", "DB10.DBW26", "Word"),
          tag(11, "Batch total", "DB10.DBD28", "DWord"),
          tag(12, "Speed setpoint", "MW50", "Word"),
        ],
      },
    ]),
  );
}

/** Seeds two PLCs for the phone harness, so the connection chips have something to switch between. */
export function seedPhoneDevices() {
  const first = JSON.parse(localStorage.getItem("s7-explorer-devices") ?? "[]");
  localStorage.setItem(
    "s7-explorer-devices",
    JSON.stringify([
      ...first,
      { id: "10000000-0000-4000-8000-000000000002", label: "Packaging", host: "192.168.10.31", port: 102, rack: 0, slot: 2, tags: [{ id: "00000000-0000-4000-8000-0000000000a1", name: "Belt speed", address: "DB1.DBD0", data_type: "Real" }] },
    ]),
  );
}
export { initDevice as seedDevices };

const CROP_PX = 54;
const sceneCss = (scale) => `
  #root > div { height: calc(100vh - ${(CROP_PX / scale).toFixed(2)}px) !important; }
  *::-webkit-scrollbar { display: none; }
  * { scrollbar-width: none; }
`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function prepare(page, scale) {
  await page.setViewport({ width: Math.round(1180 / scale), height: Math.round(740 / scale), deviceScaleFactor: 2 * scale });
  await page.addStyleTag({ content: sceneCss(scale) });
}

async function clickButton(page, text) {
  const ok = await page.evaluate((t) => {
    const b = [...document.querySelectorAll("button")].find((el) => el.textContent.trim() === t && !el.disabled);
    if (!b) return false;
    b.click();
    return true;
  }, text);
  if (!ok) throw new Error(`button "${text}" not found`);
}

async function clickTab(page, text) {
  const ok = await page.evaluate((t) => {
    const b = [...document.querySelectorAll('[role="tab"]')].find((el) => el.textContent.trim().startsWith(t));
    if (!b) return false;
    b.click();
    return true;
  }, text);
  if (!ok) throw new Error(`tab "${text}" not found`);
}

async function connect(page) {
  await clickButton(page, "Connect");
  await page.waitForFunction(() => document.body.textContent.includes("Polls") || document.querySelector("tbody td")?.textContent, { timeout: 8000 });
  await sleep(1500);
}

async function setPollMs(page, ms) {
  await page.evaluate((v) => {
    const el = document.querySelector('input[id^="poll-"]');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, String(v));
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }, ms);
  await sleep(300);
}

/** Gives the main (top) pane more height than the comm log / trend pane below it. */
async function layout(page, topFlex) {
  await page.evaluate((r) => {
    const tabs = document.querySelector('[aria-label="Bottom views"]');
    const bottom = tabs.parentElement;
    bottom.style.flex = "1 1 0";
    bottom.previousElementSibling.style.flex = `${r} 1 0`;
  }, topFlex);
}

const common = { init: initDevice, settleMs: 500 };

export const uiScale = 1.0;

export const scenes = [
  {
    id: "live-tags",
    headline: "Watch PLC values <em>live</em>",
    sub: "Poll data block, input, output and marker addresses and read them as Real, Word, DWord or Bool.",
    ...common,
    async setup(page) {
      await prepare(page, 1.0);
      await layout(page, 2.1);
      await connect(page);
      await sleep(2500);
    },
  },
  {
    id: "diagnostics",
    headline: "See CPU health <em>at a glance</em>",
    sub: "Operating mode, module type, firmware and the diagnostic buffer, straight from the controller.",
    ...common,
    async setup(page) {
      await prepare(page, 1.0);
      await layout(page, 3);
      await connect(page);
      await clickTab(page, "Diagnostics");
      await sleep(1500);
      await page.evaluate(() => {
        const el = [...document.querySelectorAll("*")].find((e) => e.children.length === 0 && e.textContent.startsWith("Diagnostic buffer"));
        el.scrollIntoView({ block: "start" });
      });
      await sleep(500);
    },
  },
  {
    id: "simulator",
    headline: "Test clients <em>without a PLC</em>",
    sub: "Run an S7 server on your Mac from a ready-made template and point any client at it.",
    ...common,
    async setup(page) {
      await prepare(page, 1.0);
      await page.evaluate(() => {
        const s = [...document.querySelectorAll("select")].find((el) => [...el.options].some((o) => o.textContent === "Select…"));
        s.setAttribute("data-template-select", "1");
      });
      await page.select("[data-template-select]", "process");
      await sleep(300);
      await layout(page, 2.1);
      await clickButton(page, "Start server");
      await sleep(1500);
      await clickButton(page, "Load from simulator");
      await sleep(600);
      await connect(page);
      await sleep(2500);
    },
  },
  {
    id: "trend",
    headline: "Catch changes <em>on a trend</em>",
    sub: "Plot any tag as it updates, with 30 second, 1 minute and 5 minute windows.",
    ...common,
    async setup(page) {
      await prepare(page, 1.0);
      await layout(page, 1.5);
      await connect(page);
      await setPollMs(page, 400);
      for (const name of ["Pump current A", "Motor temp C", "Tank level %", "Flow m3/h"]) {
        await page.click(`button[aria-label="Add ${name} to trend"]`);
      }
      await clickTab(page, "Trend");
      await clickButton(page, "30s");
      await sleep(24000);
      await setPollMs(page, 1000);
      await sleep(3500);
    },
  },
  {
    id: "blocks",
    headline: "Manage program blocks <em>over the network</em>",
    sub: "List organization, function and data blocks on the controller and save any block to a file.",
    ...common,
    async setup(page) {
      await prepare(page, 1.0);
      await layout(page, 2.4);
      await connect(page);
      await clickTab(page, "Blocks");
      await sleep(1500);
    },
  },
];
