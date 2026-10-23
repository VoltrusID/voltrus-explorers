// App Store scenes for EtherNet/IP Explorer. Mock data models a CompactLogix 5380 on a packaging line,
// the devices a ListIdentity broadcast would find, and the packets a session produces.
//
// Mock command shapes follow apps/enip-explorer/src-tauri/src/lib.rs and crates/mt-enip/src/types.rs
// (CipValue serializes as {Real: 1.2}, {Dint: 3}, {Bool: true}; CipDataType as a lowercase string).

import { pickOption } from "../pick.mjs";

export const uiScale = 1.1;

// [name, type, value fn key]
const TAGS = [
  ["LineSpeed_SP", "real", { base: 120, amp: 0, dec: 1 }],
  ["LineSpeed_PV", "real", { base: 118.6, amp: 1.4, per: 9, dec: 1 }],
  ["Conv1.Running", "bool", { b: true }],
  ["Conv1.Fault", "bool", { b: false }],
  ["Conv1.MotorCurrent", "real", { base: 6.8, amp: 0.4, per: 7, dec: 2 }],
  ["Conv1.MotorTemp", "real", { base: 58.2, amp: 0.6, per: 40, dec: 1 }],
  ["Conv2.Running", "bool", { b: true }],
  ["Conv2.Fault", "bool", { b: false }],
  ["Conv2.MotorCurrent", "real", { base: 5.9, amp: 0.3, per: 8, dec: 2 }],
  ["Filler.State", "dint", { d: 3 }],
  ["Filler.BottleCount", "dint", { counter: 48210, rate: 7 }],
  ["Filler.RejectCount", "dint", { counter: 312, rate: 0.1 }],
  ["Filler.FillLevel", "real", { base: 74.3, amp: 6, per: 5, dec: 1 }],
  ["Filler.ProductTemp", "real", { base: 4.2, amp: 0.15, per: 30, dec: 1 }],
  ["Capper.Torque", "real", { base: 1.82, amp: 0.05, per: 6, dec: 2 }],
  ["Capper.CapsRemaining", "dint", { counter: 2650, rate: -7 }],
  ["Labeler.Running", "bool", { b: true }],
  ["Labeler.RollLength", "real", { base: 412.5, amp: 0, dec: 1 }],
  ["Palletizer.LayerCount", "dint", { d: 4 }],
  ["Palletizer.CycleTime", "real", { base: 11.4, amp: 0.2, per: 12, dec: 1 }],
  ["Packaging.OEE", "real", { base: 87.6, amp: 0.3, per: 50, dec: 1 }],
  ["Alarm.Active", "bool", { b: false }],
  ["Alarm.Count", "dint", { d: 0 }],
  ["ShiftNumber", "dint", { d: 2 }],
];

const DEVICES = [
  ["10.20.4.10", "1756-EN2T/D", 1, "Rockwell Automation/Allen-Bradley", 12, 166, 11, 1, 0x60a1f3c2],
  ["10.20.4.21", "5069-L330ER CompactLogix 5380", 1, "Rockwell Automation/Allen-Bradley", 14, 190, 32, 11, 0x61b2d4e9],
  ["10.20.4.35", "2080-LC50-24QWB Micro850", 1, "Rockwell Automation/Allen-Bradley", 14, 2171, 12, 1, 0x60c4a771],
  ["10.20.4.52", "PowerFlex 755", 1, "Rockwell Automation/Allen-Bradley", 2, 1040, 13, 2, 0x5fa09b36],
  ["10.20.5.14", "FLEN-8M12 Block I/O", 48, "Turck Inc.", 7, 2, 3, 6, 0x00a3c120],
  ["10.20.5.31", "XGS1 Gateway", 14, "Banner Engineering", 12, 21, 2, 4, 0x01f2b934],
  ["10.20.6.8", "ACS880 FENA-21", 281, "ABB Drives", 2, 1, 2, 3, 0x0188ce44],
  ["10.20.6.19", "SmartPAC Pro IO", 100, "Cognex Corporation", 12, 120, 5, 1, 0x02c817d0],
].map(([host, product_name, vendor_id, vendor_name, device_type, product_code, rmaj, rmin, serial]) => ({
  host,
  port: 44818,
  vendor_id,
  vendor_name,
  device_type,
  product_code,
  revision_major: rmaj,
  revision_minor: rmin,
  status: 0x0030,
  serial_number: serial,
  product_name,
  state: 3,
}));

const PACKETS = [
  ["Tx", 0x0065, 0, [0x65, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]],
  ["Rx", 0x0065, 0x1a2b3c4d, [0x65, 0, 4, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]],
  ["Tx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x1a, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0e, 0x03, 0x20, 0x01, 0x24, 0x01, 0x30, 0x01]],
  ["Rx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x1c, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x8e, 0, 0, 0, 0x01, 0x00]],
  ["Tx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x2a, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x4c, 0x08, 0x91, 0x0c, 0x4c, 0x69, 0x6e, 0x65, 0x53, 0x70, 0x65, 0x65, 0x64]],
  ["Rx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x1e, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xcc, 0, 0, 0, 0xca, 0x00, 0x3d, 0x2b, 0xed, 0x42]],
  ["Tx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x2c, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x4c, 0x0a, 0x91, 0x08, 0x46, 0x69, 0x6c, 0x6c, 0x4c, 0x65, 0x76, 0x65, 0x6c]],
  ["Rx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x1e, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xcc, 0, 0, 0, 0xca, 0x00, 0x7b, 0x14, 0x94, 0x42]],
  ["Tx", 0x0063, 0, [0x63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  ["Tx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x26, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x4c, 0x06, 0x91, 0x04, 0x4f, 0x45, 0x45, 0x00]],
  ["Rx", 0x006f, 0x1a2b3c4d, [0x6f, 0, 0x1e, 0, 0x4d, 0x3c, 0x2b, 0x1a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xcc, 0, 0, 0, 0xca, 0x00, 0x71, 0x3d, 0xaf, 0x42]],
];

const SIM_TAGS = [
  ["Program:Main.Motor1.Speed", "real", { base: 1450, amp: 12, per: 15, dec: 1 }],
  ["Program:Main.Motor1.Current", "real", { base: 12.4, amp: 0.8, per: 12, dec: 1 }],
  ["Program:Main.Motor1.Torque", "real", { base: 45.8, amp: 2.5, per: 14, dec: 1 }],
  ["Program:Main.Motor1.Temp", "real", { base: 67.2, amp: 1.5, per: 60, dec: 1 }],
  ["Program:Main.Motor1.Power", "real", { base: 7.5, amp: 0.4, per: 13, dec: 2 }],
  ["Program:Main.Motor1.Runtime", "dint", { d: 15234 }],
  ["Program:Main.Motor1.Frequency", "real", { base: 50, amp: 0.02, per: 10, dec: 2 }],
  ["Program:Main.Motor1.Status.Running", "bool", { b: true }],
  ["Program:Main.Motor1.Status.Faulted", "bool", { b: false }],
  ["Program:Main.Motor1.Status.Overloaded", "bool", { b: false }],
  ["Program:Main.Motor1.Mode", "dint", { d: 2 }],
  ["Program:Main.Motor1.SpeedReference", "real", { base: 1450, amp: 0, dec: 0 }],
];

function mockMain(D) {
  const T0 = Date.UTC(2026, 9, 21, 10, 14, 0);
  const realNow = Date.now.bind(Date);
  const r0 = realNow();
  Date.now = () => T0 + (realNow() - r0);
  // Headless screen == viewport, which the Toolbar reads as native fullscreen and drops the traffic-light inset.
  Object.defineProperty(window.screen, "width", { get: () => 2560 });
  Object.defineProperty(window.screen, "height", { get: () => 1600 });

  const cbs = new Map();
  const listeners = new Map();
  let nextCb = 1;
  let nextEv = 1;
  window.__emit = (event, payload) => {
    for (const l of listeners.get(event) ?? []) {
      const fn = cbs.get(l.handler);
      if (fn) fn({ event, id: l.id, payload });
    }
  };

  const valueOf = (spec, type) => {
    const t = (Date.now() - T0) / 1000;
    if (type === "bool") return { Bool: !!spec.b };
    if (spec.d !== undefined) return { Dint: spec.d };
    if (spec.counter !== undefined) return { Dint: Math.round(spec.counter + spec.rate * t) };
    const v = spec.base + (spec.amp ? spec.amp * Math.sin((t / spec.per) * Math.PI * 2) : 0);
    return { Real: Number(v.toFixed(spec.dec ?? 1)) };
  };
  const tagMeta = D.tags.map(([name, type]) => ({ name, data_type: type, element_count: 1 }));
  const specOf = (name) => D.tags.find((t) => t[0] === name);

  const sim = { running: false };
  const bytesU16 = (n) => [n & 255, (n >> 8) & 255];
  const str = (s) => [s.length, ...[...s].map((c) => c.charCodeAt(0))];
  const dev = D.devices[1];
  const attrs = {
    "1:1:1": bytesU16(dev.vendor_id),
    "1:1:2": bytesU16(dev.device_type),
    "1:1:3": bytesU16(dev.product_code),
    "1:1:4": [dev.revision_major, dev.revision_minor],
    "1:1:5": bytesU16(dev.status),
    "1:1:6": [dev.serial_number & 255, (dev.serial_number >> 8) & 255, (dev.serial_number >> 16) & 255, (dev.serial_number >>> 24) & 255],
    "1:1:7": str(dev.product_name),
    "245:1:1": [0x03, 0, 0, 0],
    "245:1:2": [0x95, 0, 0, 0],
    "245:1:3": [0x01, 0, 0, 0],
    "245:1:5": [10, 20, 4, 21, 255, 255, 255, 0, 10, 20, 4, 1],
    "245:1:6": [15, 0, 0, 0, ...[..."cl5380-packaging"].map((c) => c.charCodeAt(0))],
    "246:1:1": [0, 0xe1, 0xf5, 0x05],
    "246:1:2": [0x0f, 0, 0, 0],
    "246:1:4": str("Port 1"),
  };

  const handlers = {
    store_status: () => ({ store: false, pro: true, reason: "build", price: null }),
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
    connect: () => null,
    disconnect: () => null,
    discover_devices: async () => {
      await new Promise((r) => setTimeout(r, 250));
      return D.devices;
    },
    get_device_identity: () => ({ ...dev }),
    browse_tags: () => tagMeta,
    read_tags: ({ tags }) => tags.map((t) => valueOf(specOf(t.name)[2], specOf(t.name)[1])),
    write_tag: () => null,
    cip_get_attribute: ({ classId, instanceId, attributeId }) => attrs[`${classId}:${instanceId}:${attributeId}`] ?? [],
    list_class_instances: ({ classId }) => (classId === 1 ? [{ instance: 1, data: attrs["1:1:1"].concat(attrs["1:1:2"], attrs["1:1:3"]) }] : []),
    cip_generic: () => [],
    sim_start: () => {
      sim.running = true;
      return null;
    },
    sim_stop: () => {
      sim.running = false;
      return null;
    },
    server_read_tags: () => D.simTags.map(([name, type, spec]) => [name, type, valueOf(spec, type)]),
    server_write_tag: () => null,
    sim_set_value: () => null,
    sim_set_bool: () => null,
    open_device_panel: () => null,
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

export const mock = `(${mockMain.toString()})(${JSON.stringify({ tags: TAGS, devices: DEVICES, simTags: SIM_TAGS })})`;

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
  if (!ok) throw new Error(`no ${selector} with text ${text}`);
  await sleep(150);
}

// Dark theme (headless prefers light) and room for the 54px the frame crops off the bottom of the window.
async function prep(page) {
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.addStyleTag({ content: "#root > div { height: calc(100vh - 50px) !important; }" });
}

async function setSidebarField(page, index, value) {
  const inputs = await page.$$("aside input");
  await inputs[index].click();
  await page.keyboard.down("Control");
  await page.keyboard.press("a");
  await page.keyboard.up("Control");
  await page.keyboard.type(String(value));
}

async function connect(page) {
  await setSidebarField(page, 0, "10.20.4.21");
  await clickText(page, "aside button", "Connect", "equals");
  await page.waitForFunction(() => [...document.querySelectorAll("aside button")].some((b) => b.textContent.trim().startsWith("Identity")), { timeout: 90000 });
  await page.waitForFunction(() => document.querySelectorAll("table tbody tr, [role=row]").length > 5, { timeout: 90000 }).catch(() => {});
  await sleep(1500);
}

async function dragHandle(page, cursor, dx, dy) {
  const box = await page.evaluate((cursor) => {
    const h = [...document.querySelectorAll("div")].find((d) => d.style.cursor === cursor && d.style.flexShrink === "0");
    const r = h.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  }, cursor);
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  await page.mouse.move(box.x + dx, box.y + dy, { steps: 6 });
  await page.mouse.up();
  await sleep(200);
}

export const scenes = [
  {
    id: "discover",
    headline: "Find every <em>EtherNet/IP</em> device",
    sub: "One broadcast lists each controller, drive and I/O block with its vendor, product, revision and serial number.",
    setup: async (page) => {
      await prep(page);
      await clickText(page, "aside button", "Discover");
      await page.waitForFunction(() => document.body.textContent.includes("8 devices found"), { timeout: 90000 });
      await sleep(300);
    },
  },
  {
    id: "tags",
    headline: "Browse and watch <em>live PLC tags</em>",
    sub: "Connect to a Logix controller, list every tag with its CIP type and watch the values update.",
    settleMs: 1200,
    setup: async (page) => {
      await prep(page);
      await connect(page);
      await dragHandle(page, "row-resize", 0, 120);
      await sleep(300);
    },
  },
  {
    id: "tree",
    headline: "Walk the <em>CIP object tree</em>",
    sub: "Identity attributes decode into readable values, and every class lists its instances.",
    setup: async (page) => {
      await prep(page);
      await connect(page);
      await dragHandle(page, "row-resize", 0, 60);
      await clickText(page, "button", "Objects", "equals");
      await page.waitForFunction(() => document.body.textContent.includes("Message Router"), { timeout: 90000 });
      // Identity starts expanded; collapse and expand again so its attributes are read.
      await clickText(page, "main span", "Identity (0x01)");
      await clickText(page, "main span", "Identity (0x01)");
      await sleep(400);
    },
  },
  {
    id: "packets",
    headline: "See every packet <em>on the wire</em>",
    sub: "Inspect each EtherNet/IP session command, filter by direction and open any frame in a hex view.",
    setup: async (page) => {
      await prep(page);
      await connect(page);
      await clickText(page, "aside button", "Packets");
      await page.evaluate((packets) => {
        const base = Date.now() - 4000;
        packets.forEach(([direction, command, session, bytes], i) => window.__emit("enip://packet", { direction, command, session, timestamp_ms: base + i * 310, bytes }));
      }, PACKETS);
      await sleep(300);
      await page.evaluate(() => {
        const rows = [...document.querySelectorAll(".vt-dialog .mono")].filter((e) => e.textContent.includes("SendRRData"));
        rows[2]?.click();
      });
      await sleep(300);
    },
  },
  {
    id: "simulator",
    headline: "Test without <em>any hardware</em>",
    sub: "Start a built-in EtherNet/IP adapter with a motor drive template and connect to it from any scanner.",
    settleMs: 1200,
    setup: async (page) => {
      await prep(page);
      await clickText(page, "button", "Adapter");
      await dragHandle(page, "row-resize", 0, 120);
      await pickOption(page, "Default tags", "MotorDrive-1 (ABB ACS880)");
      await sleep(150);
      await clickText(page, "aside button", "Start simulator");
      await page.waitForFunction(() => document.body.textContent.includes("Motor1.Speed"), { timeout: 90000 });
      await sleep(800);
    },
  },
];

/** One saved decoder, so the decoder selects list more than their "none" row. */
const SEED_DECODERS = `localStorage.setItem("enip.decoders", ${JSON.stringify(JSON.stringify([{ id: "d1", name: "Drive status", fields: [{ name: "state", type: "uint16", size: 0 }, { name: "speed", type: "real", size: 0 }] }]))});`;

/** Extra states for render-selects.mjs: selects the App Store scenes never show enabled. */
export const selectScenes = [
  {
    id: "adapter-ready",
    async setup(page) {
      await prep(page);
      await clickText(page, "button", "Adapter");
      await sleep(400);
    },
  },
  {
    id: "cip-browser",
    init: SEED_DECODERS,
    async setup(page) {
      await prep(page);
      await connect(page);
      await clickText(page, "aside button", "CIP Object");
      await page.waitForSelector(".vt-dialog");
      await clickText(page, ".vt-dialog button, .vt-dialog [role=tab]", "Console");
      await clickText(page, ".vt-dialog button", "List Inst");
      await page.waitForFunction(() => document.querySelector(".vt-dialog")?.textContent.includes("Instances (1)"), { timeout: 10000 });
      await sleep(300);
    },
  },
  {
    id: "decoder-editor",
    init: SEED_DECODERS,
    async setup(page) {
      await prep(page);
      await connect(page);
      await clickText(page, "aside button", "CIP Object");
      await page.waitForSelector(".vt-dialog");
      await clickText(page, ".vt-dialog button, .vt-dialog [role=tab]", "Console");
      await clickText(page, ".vt-dialog button", "Decoders");
      await page.waitForSelector('[aria-label="Field type"]');
      await sleep(300);
    },
  },
  {
    id: "io-pump",
    init: SEED_DECODERS,
    async setup(page) {
      await prep(page);
      await connect(page);
      await clickText(page, "aside button", "IO");
      await page.waitForSelector(".vt-dialog");
      await sleep(300);
    },
  },
];
