// App Store scenes for BACnet Explorer. Mock data models one AHU controller (device 2001) on a
// building network, plus the other devices a Who-Is broadcast would find.
//
// Mock command shapes follow apps/bacnet-explorer/src-tauri/src/main.rs and crates/mt-bacnet
// (read_object_properties returns the 12 properties the Rust side reads; BitString status flags
// decode to BacnetValue::Null on the Rust side, so the inspector really shows "—" for them).

import { connectReviewScene } from "../connect-review.mjs";

export const uiScale = 1.1;

const U = {
  degC: ["Degrees Celsius", "°C", 62],
  pct: ["Percent", "%", 98],
  pa: ["Pascals", "Pa", 53],
  ppm: ["Parts Per Million", "ppm", 96],
  rh: ["Percent Relative Humidity", "%RH", 29],
  lps: ["Litres Per Second", "L/s", 87],
};

// [type, instance, name, description, presentValue, unit, hiLimit, loLimit, covIncrement]
const OBJ = [
  ["device", 2001, "AHU-01", "Air handling unit 1, roof plant"],
  ["analogInput", 1, "SA-TEMP", "Supply air temperature", 13.2, U.degC, 18, 8, 0.2],
  ["analogInput", 2, "RA-TEMP", "Return air temperature", 23.4, U.degC, 28, 18, 0.2],
  ["analogInput", 3, "OA-TEMP", "Outdoor air temperature", 29.6, U.degC, 45, -10, 0.2],
  ["analogInput", 4, "MA-TEMP", "Mixed air temperature", 19.8, U.degC, 30, 5, 0.2],
  ["analogInput", 5, "SA-PRESS", "Supply duct static pressure", 248, U.pa, 400, 50, 5],
  ["analogInput", 6, "SA-FLOW", "Supply airflow", 1840, U.lps, 2600, 600, 25],
  ["analogInput", 7, "RA-CO2", "Return air CO2", 642, U.ppm, 1000, 400, 10],
  ["analogInput", 8, "RA-RH", "Return air relative humidity", 47.5, U.rh, 65, 30, 1],
  ["analogOutput", 1, "CHWV-CMD", "Chilled water valve command", 62, U.pct, null, null, 1],
  ["analogOutput", 2, "HWV-CMD", "Hot water valve command", 0, U.pct, null, null, 1],
  ["analogOutput", 3, "OAD-CMD", "Outdoor air damper command", 28, U.pct, null, null, 1],
  ["analogOutput", 4, "SF-SPD-CMD", "Supply fan speed command", 74, U.pct, null, null, 1],
  ["analogValue", 1, "SA-TEMP-SP", "Supply air temperature setpoint", 13, U.degC, 18, 8, 0.1],
  ["analogValue", 2, "SA-PRESS-SP", "Duct static pressure setpoint", 250, U.pa, 400, 50, 5],
  ["analogValue", 3, "RA-CO2-SP", "CO2 demand ventilation setpoint", 800, U.ppm, 1000, 400, 10],
  ["analogValue", 4, "OAD-MIN", "Minimum outdoor air damper position", 20, U.pct, 100, 0, 1],
  ["binaryInput", 1, "SF-STATUS", "Supply fan run status", 1],
  ["binaryInput", 2, "FILTER-DP-ALM", "Filter differential pressure alarm", 1],
  ["binaryInput", 3, "FREEZESTAT", "Coil freeze protection", 0],
  ["binaryInput", 4, "SF-OVERLOAD", "Supply fan motor overload", 0],
  ["binaryOutput", 1, "SF-CMD", "Supply fan start command", 1],
  ["binaryOutput", 2, "CHWP-CMD", "Chilled water pump command", 1],
  ["binaryValue", 1, "OCC-MODE", "Occupied mode", 1],
  ["multiStateValue", 1, "OP-MODE", "Operating mode (1 off, 2 cooling, 3 heating)", 2],
  ["schedule", 1, "OCC-SCHED", "Weekly occupancy schedule"],
  ["trendLog", 1, "SA-TEMP-LOG", "Supply air temperature log"],
];

const DEVICES = [
  [2001, "10.20.4.21", "Johnson Controls"],
  [3010, "10.20.4.30", "Schneider Electric"],
  [4101, "10.20.5.12", "Siemens"],
  [5003, "10.20.5.40", "Honeywell"],
  [6120, "10.20.6.8", "Trane"],
  [7002, "10.20.6.17", "Distech Controls"],
  [8015, "10.20.7.3", "Delta Controls"],
  [9004, "10.20.3.5", "Tridium"],
].map(([id, ip, vendor]) => ({
  device_instance: id,
  ip,
  port: 47808,
  vendor_name: vendor,
  model_name: null,
  firmware_version: null,
  object_list_count: null,
}));

const ALL3 = "ToOffNormal, ToFault, ToNormal";
// object_type, instance, state, acked, [to-offnormal, to-fault, to-normal] time stamps
const ALARMS = [
  ["analogInput", 7, "highLimit", "ToFault, ToNormal", ["09:12:44.00", "14:03:10.00", "07:58:32.00"]],
  ["binaryInput", 2, "offnormal", "ToFault, ToNormal", ["08:47:05.00", "11:26:51.00", "16:40:09.00"]],
  ["analogInput", 5, "fault", "ToOffNormal, ToNormal", ["22:15:30.00", "06:31:12.00", "22:41:03.00"]],
  ["analogInput", 4, "lowLimit", ALL3, ["05:52:18.00", "03:09:44.00", "06:20:47.00"]],
  ["binaryInput", 3, "normal", "ToOffNormal, ToFault", ["18:04:21.00", "07:30:00.00", "18:09:55.00"]],
  ["analogInput", 8, "highLimit", ALL3, ["13:22:37.00", "10:15:08.00", "15:48:12.00"]],
];

const SIM_POINTS = [
  { key: "supply_temp", label: "SupplyTemp", object_type: "analogInput", object_instance: 1, base: 22.5, variance: 0.4, period: 30000, min: 18, max: 28, decimals: 1, unit: "°C", writable: false, kind: "analog" },
  { key: "return_temp", label: "ReturnTemp", object_type: "analogInput", object_instance: 2, base: 24.1, variance: 0.3, period: 35000, min: 20, max: 28, decimals: 1, unit: "°C", writable: false, kind: "analog" },
  { key: "outdoor_temp", label: "OutdoorTemp", object_type: "analogInput", object_instance: 3, base: 18.3, variance: 0.2, period: 60000, min: 10, max: 25, decimals: 1, unit: "°C", writable: false, kind: "analog" },
  { key: "supply_pressure", label: "SupplyPressure", object_type: "analogInput", object_instance: 4, base: 245, variance: 8, period: 20000, min: 200, max: 300, decimals: 0, unit: "Pa", writable: false, kind: "analog" },
  { key: "fan_speed", label: "FanSpeed", object_type: "analogInput", object_instance: 5, base: 1450, variance: 15, period: 25000, min: 1400, max: 1500, decimals: 0, unit: "RPM", writable: false, kind: "analog" },
  { key: "fan_status", label: "FanStatus", object_type: "binaryInput", object_instance: 1, base: 1, variance: 0, period: 100000, min: null, max: null, decimals: 0, unit: null, writable: false, kind: "bool" },
  { key: "filter_status", label: "FilterOK", object_type: "binaryInput", object_instance: 2, base: 1, variance: 0, period: 100000, min: null, max: null, decimals: 0, unit: null, writable: false, kind: "bool" },
  { key: "setpoint", label: "TempSetpoint", object_type: "analogValue", object_instance: 1, base: 23, variance: 0, period: 100000, min: 16, max: 30, decimals: 1, unit: "°C", writable: true, kind: "analog" },
  { key: "damper_pos", label: "DamperPosition", object_type: "analogValue", object_instance: 2, base: 45, variance: 5, period: 40000, min: 0, max: 100, decimals: 1, unit: "%", writable: true, kind: "analog" },
];

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
    for (const l of listeners.get(event) ?? []) {
      const fn = cbs.get(l.handler);
      if (fn) fn({ event, id: l.id, payload });
    }
  };

  const objects = D.objects.map(([type, instance, name, desc, pv, unit, hi, lo, cov]) => ({ type, instance, name, desc, pv, unit, hi, lo, cov }));
  const find = (t, i) => objects.find((o) => o.type === t && o.instance === i);

  const P = (id, name, value, raw) => ({ property_name: name, property_identifier: id, value, raw_value: raw });
  const readProps = (t, i) => {
    const o = find(t, i);
    if (!o) throw "Error(1, 31)";
    const analog = t.startsWith("analog");
    const props = [P(77, "objectName", { String: o.name }, o.name), P(28, "description", { String: o.desc }, o.desc)];
    if (o.pv === undefined) {
      return { object_type: t, object_instance: i, object_name: o.name, description: o.desc, present_value: null, units: null, units_symbol: null, status_flags: null, reliability: null, out_of_service: null, properties: props };
    }
    if (analog) props.push(P(85, "presentValue", { Real: o.pv }, o.pv.toFixed(2)));
    else props.push(P(85, "presentValue", { Enum: o.pv }, "Enum(" + o.pv + ")"));
    props.push(P(111, "statusFlags", "Null", "Bits(0000)"));
    if (analog) props.push(P(117, "units", { Enum: o.unit[2] }, o.unit[0]));
    props.push(P(103, "reliability", { Enum: 0 }, "Enum(0)"), P(81, "outOfService", { Boolean: false }, "False"), P(36, "eventState", { Enum: 0 }, "Enum(0)"), P(17, "notificationClass", { Unsigned: 1 }, "1"));
    if (analog && o.hi !== null) props.push(P(45, "highLimit", { Real: o.hi }, o.hi.toFixed(2)), P(59, "lowLimit", { Real: o.lo }, o.lo.toFixed(2)));
    if (analog) props.push(P(22, "covIncrement", { Real: o.cov }, o.cov.toFixed(2)));
    return {
      object_type: t,
      object_instance: i,
      object_name: o.name,
      description: o.desc,
      present_value: o.pv.toFixed(2),
      units: analog ? o.unit[0] : null,
      units_symbol: analog ? o.unit[1] : null,
      status_flags: "Bits(0000)",
      reliability: "Enum(0)",
      out_of_service: false,
      properties: props,
    };
  };

  const bits = (s) => s;
  const eventEntries = () =>
    D.alarms.map(([t, i, state, acked, stamps]) => ({
      object_identifier: t + ":" + i,
      object_type: t,
      object_type_id: t === "analogInput" ? 0 : 3,
      object_instance: i,
      event_state: state,
      acknowledged_transitions: bits(acked),
      event_enable: "ToOffNormal, ToFault, ToNormal",
      event_time_stamps: stamps,
      transition_time_stamps: stamps.map(() => ({ kind: "sequenceNumber", sequence_number: 1 })),
      notify_type: "alarm",
      event_priorities: [64, 64, 128],
    }));

  const sim = { running: false, port: 47808, device: 100, overrides: {}, step: 0 };
  const simSnapshot = () => ({
    device_id: sim.device,
    port: sim.port,
    running: true,
    points: D.simPoints.map((p) => {
      let v = p.base;
      if (p.variance) v = p.base + p.variance * Math.sin((sim.step * 1000) / p.period * 2 * Math.PI + p.object_instance) + 0.3 * p.variance * Math.sin(sim.step * 1.7 + p.object_instance * 3);
      if (sim.overrides[p.key] !== undefined) v = sim.overrides[p.key];
      if (p.kind === "bool") return { ...p, value: v, display: v >= 0.5 ? "Active" : "Inactive" };
      return { ...p, value: +v.toFixed(p.decimals), display: v.toFixed(p.decimals) + (p.unit ? " " + p.unit : "") };
    }),
  });
  window.__simTick = (n) => {
    for (let k = 0; k < n; k++) {
      sim.step += 1;
      window.__skew += 1000;
      window.__emit("sim-tick", simSnapshot());
    }
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
    read_tags: () => [],
    write_tag: () => null,
    discover: () => {
      if (window.__discoverFail) throw window.__discoverFail;
      return window.__discoverEmpty ? [] : D.devices;
    },
    platform: () => window.__platform ?? "macos",
    browse_objects: () => objects.map((o) => ({ object_type: o.type, object_instance: o.instance, object_name: o.name })),
    read_object_properties: ({ objectType, objectInstance }) => readProps(objectType, objectInstance),
    inspect_device_cmd: () => ({
      device_instance: 2001, object_name: "AHU-01", vendor_name: "Johnson Controls", vendor_id: 5, model_name: "FEC-2611", firmware_revision: "10.4.1", application_software_version: "AHU_v3.2",
      description: "Air handling unit 1, roof plant", location: "Roof plant room", protocol_version: 1, protocol_revision: 14, max_apdu_length: 1476, segmentation_supported: "Enum(3)", apdu_timeout: 3000,
      number_of_apdu_retries: 3, device_address_binding: null, object_count: objects.length, system_status: "Enum(0)", local_time: "10:14:00.00", local_date: "2026-10-21 (DoW:3)", database_revision: 42,
      services_supported: null, object_types_supported: null,
    }),
    start_cov_listener: () => null,
    subscribe_cov_cmd: () => null,
    unsubscribe_cov_cmd: () => null,
    get_alarm_summary: () => D.alarms.map(([t, i, state, acked]) => ({ object_identifier: t + ":" + i, object_type: t, object_instance: i, alarm_state: state, acked_transitions: acked })),
    get_event_information: () => ({ entries: eventEntries(), more_events: false }),
    acknowledge_alarm_cmd: () => null,
    sim_start: ({ deviceId, port }) => {
      sim.running = true;
      sim.device = deviceId;
      sim.port = port;
      return { device_id: deviceId, port };
    },
    sim_stop: () => {
      sim.running = false;
      window.__emit("sim-stopped", null);
      return null;
    },
    sim_status: () => sim.running,
    open_device_panel: () => null,
    sim_set_value: ({ key, value }) => {
      sim.overrides[key] = value;
      window.__emit("sim-written", { key, value });
      return null;
    },
    sim_set_bool: ({ key, active }) => {
      sim.overrides[key] = active ? 1 : 0;
      window.__emit("sim-written", { key, value: active ? 1 : 0 });
      return null;
    },
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

export const mock = `(${mockMain.toString()})(${JSON.stringify({ objects: OBJ, devices: DEVICES, alarms: ALARMS, simPoints: SIM_POINTS })})`;

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
  await sleep(120);
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

async function connect(page, ip = "10.20.4.21", deviceId = 2001) {
  await setSidebarField(page, 0, ip);
  await setSidebarField(page, 2, deviceId);
  await clickText(page, "aside button", "Connect", "equals");
  await page.waitForFunction(() => [...document.querySelectorAll("aside button")].some((b) => b.textContent.trim() === "Alarms"), { timeout: 5000 });
  await sleep(150);
}

async function browse(page) {
  await clickText(page, "button", "Browse objects", "equals");
  await page.waitForSelector(".ob-group", { timeout: 5000 });
  await sleep(150);
}

const expand = (page, label) => clickText(page, ".ob-group", label);

async function selectObject(page, name) {
  const ok = await page.evaluate((name) => {
    const row = [...document.querySelectorAll(".ob-row")].find((r) => r.querySelector("span")?.textContent === name);
    if (!row) return false;
    row.click();
    return true;
  }, name);
  if (!ok) throw new Error("no object row " + name);
  await sleep(250);
}

async function addTrend(page, names) {
  for (const name of names) {
    const ok = await page.evaluate((name) => {
      const row = [...document.querySelectorAll(".ob-row")].find((r) => r.querySelector("span")?.textContent === name);
      const btn = row?.querySelector('button[aria-label="Add to trend"]');
      if (!btn) return false;
      btn.click();
      return true;
    }, name);
    if (!ok) throw new Error("no trend button for " + name);
    await sleep(120);
  }
  // Switch every series to COV and use the 5 minute window.
  await page.evaluate(() => document.querySelectorAll('button[title^="Data source: COV"]').forEach((b) => b.click()));
  await sleep(150);
  await page.evaluate(() => document.querySelector('button[aria-label="Show last 5 minutes"]')?.click());
  await sleep(150);
}

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Feed COV notifications (3 s apart, only when the value moved) so the chart has ~5 minutes of history.
const TREND = [
  { inst: 1, base: 13.2, amp: 0.45, period: 48, drift: 0, noise: 0.06 },
  { inst: 2, base: 23.1, amp: 0.12, period: 80, drift: 0.35, noise: 0.04 },
  { inst: 4, base: 19.6, amp: 0.5, period: 62, drift: 0.2, noise: 0.07 },
  { inst: 3, base: 29.2, amp: 0.1, period: 120, drift: 0.55, noise: 0.03 },
];

async function feedCov(page, count = TREND.length, steps = 96) {
  const rand = rng(7);
  const series = TREND.slice(0, count);
  const frames = [];
  const last = series.map(() => null);
  for (let i = 0; i < steps; i++) {
    const frame = [];
    series.forEach((s, k) => {
      const v = +(s.base + s.drift * (i / steps) + s.amp * Math.sin(((i * 3) / s.period) * 2 * Math.PI + k) + (rand() - 0.5) * 2 * s.noise).toFixed(1);
      if (v !== last[k]) {
        last[k] = v;
        frame.push([s.inst, v]);
      }
    });
    frames.push(frame);
  }
  await page.evaluate(async (frames) => {
    for (const frame of frames) {
      window.__skew += 3000;
      for (const [inst, v] of frame) {
        window.__emit("cov-notification", {
          subscriber_process_identifier: 1,
          initiating_device_identifier: 2001,
          monitored_object_type: "analogInput",
          monitored_object_instance: inst,
          time_remaining: 3500,
          values: [{ property_name: "presentValue", value: { Real: v }, raw_value: v.toFixed(2) }],
          timestamp: Date.now(),
        });
      }
      await new Promise((r) => setTimeout(r, 2));
    }
  }, frames);
  await sleep(300);
}

async function narrowSidebar(page, by) {
  const box = await page.evaluate(() => {
    const h = [...document.querySelectorAll("div")].find((d) => d.style.cursor === "col-resize");
    const r = h.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  });
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  await page.mouse.move(box.x - by, box.y, { steps: 6 });
  await page.mouse.up();
  await sleep(200);
}

async function resizeTrend(page, deltaUp) {
  const box = await page.evaluate(() => {
    const h = [...document.querySelectorAll("main div")].find((d) => d.style.cursor === "row-resize");
    const r = h.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  });
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  await page.mouse.move(box.x, box.y - deltaUp, { steps: 8 });
  await page.mouse.up();
  await sleep(250);
}

// ───────────────────────────── scenes ─────────────────────────────

export const scenes = [
  {
    id: "browse",
    headline: "Browse any <em>BACnet</em> controller",
    sub: "Expand the object list and read present value, units, limits and event state for every point.",
    settleMs: 800,
    setup: async (page) => {
      await connect(page);
      await browse(page);
      await expand(page, "Analog Input");
      await selectObject(page, "SA-TEMP");
      await addTrend(page, ["SA-TEMP", "RA-TEMP", "MA-TEMP"]);
      await resizeTrend(page, -45);
      await feedCov(page, 3, 70);
    },
  },
  {
    id: "discover",
    headline: "Find every <em>device</em> on the network",
    sub: "A Who-Is broadcast lists each controller with its device ID, address and vendor. Connect in one click.",
    setup: async (page) => {
      await connect(page);
      await browse(page);
      await expand(page, "Analog Input");
      await selectObject(page, "RA-CO2");
      await clickText(page, "aside button", "Discover devices");
      await page.waitForSelector(".vt-dialog table", { timeout: 5000 });
      await sleep(300);
    },
  },
  {
    id: "trend",
    headline: "Trend points as they <em>change</em>",
    sub: "Subscribe to COV notifications and watch several points on one live chart, over 30 seconds to 5 minutes.",
    setup: async (page) => {
      await connect(page);
      await browse(page);
      await expand(page, "Analog Input");
      await selectObject(page, "SA-TEMP");
      await addTrend(page, ["SA-TEMP", "RA-TEMP", "MA-TEMP", "OA-TEMP"]);
      await resizeTrend(page, 110);
      await feedCov(page, 4, 96);
    },
  },
  {
    id: "alarms",
    headline: "Check alarms, then <em>acknowledge</em>",
    sub: "Read the device's alarm summary, open each object's event timestamps and acknowledge with your name.",
    setup: async (page) => {
      await connect(page);
      await narrowSidebar(page, 80);
      await clickText(page, "aside button", "Alarms");
      await sleep(200);
      await clickText(page, "main button", "Refresh");
      await page.waitForSelector("main table", { timeout: 5000 });
      await sleep(200);
      await clickText(page, "main tbody tr:nth-child(1) button", "Events");
      await page.waitForFunction(() => document.querySelectorAll("main table").length >= 2, { timeout: 5000 });
      await sleep(300);
    },
  },
  {
    id: "simulator",
    headline: "Test without <em>any hardware</em>",
    sub: "Run a virtual AHU on UDP 47808, then browse it, write setpoints and trend it like a real controller.",
    setup: async (page) => {
      await page.select("aside select", "ahu");
      await sleep(150);
      await clickText(page, "aside button", "Start simulation");
      await sleep(250);
      await page.evaluate(() => window.__simTick(40));
      await page.evaluate(() => { const a = document.querySelector("aside"); a.scrollTop = a.scrollHeight; });
      await sleep(200);
    },
  },
];

for (const s of scenes) {
  const run = s.setup;
  s.setup = async (page) => {
    await prep(page);
    await run(page);
  };
}

// ───────────────────────────── iPhone / iPad scenes ─────────────────────────────
// Rendered with `bun marketing/screenshots/render.mjs bacnet-explorer --device=iphone|ipad`. They drive the phone tab
// layout (iPhone) and the two-pane layout (iPad). Everything shown works without Apple's multicast entitlement:
// connecting by IP address, browsing, reading, writing, trending, alarms and the built-in simulator. Device scanning is
// never shown.

const iosInit = `window.__platform = "ios";`;

// Chromium reports 0 for env(safe-area-inset-*); draw what a device has: a status bar, a taller title bar, a home-indicator gap.
function iosChromeCss(device) {
  const top = device === "iphone" ? 54 : 24;
  const bottom = device === "iphone" ? 28 : 20;
  return `
    *::-webkit-scrollbar { display: none; } * { scrollbar-width: none; }
    .bx-root .vt-titlebar { height: ${top + 48}px !important; padding-top: ${top}px !important; }
    .bx-tabbar { padding-bottom: ${bottom}px !important; }
    .bx-root .vt-statusbar { height: ${28 + (device === "ipad" ? bottom : 0)}px !important; padding-bottom: ${device === "ipad" ? bottom : 0}px !important; }
    .bx-sheet header { padding-top: ${top + 12}px !important; }
    .bx-sheet footer { padding-bottom: ${bottom + 8}px !important; }
    .vt-dialog__footer { padding-bottom: ${12 + (device === "iphone" ? bottom : 0)}px !important; }
    #ios-status { position: fixed; top: 0; left: 0; right: 0; height: ${top}px; z-index: 1000; pointer-events: none;
      display: flex; align-items: ${device === "iphone" ? "center" : "flex-end"}; justify-content: space-between;
      padding: ${device === "iphone" ? "6px 34px 0 42px" : "0 22px 4px"}; color: #fff; font: 600 ${device === "iphone" ? 17 : 14}px Inter, sans-serif; }
    #ios-status .icons { display: flex; align-items: center; gap: 6px; }
  `;
}

const STATUS_HTML = `
  <span>9:41</span>
  <span class="icons">
    <svg width="18" height="12" viewBox="0 0 18 12" fill="#fff"><rect x="0" y="8" width="3" height="4" rx="1"/><rect x="5" y="5.5" width="3" height="6.5" rx="1"/><rect x="10" y="3" width="3" height="9" rx="1"/><rect x="15" y="0" width="3" height="12" rx="1"/></svg>
    <svg width="16" height="12" viewBox="0 0 16 12" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round"><path d="M1.5 4.2a9.5 9.5 0 0 1 13 0"/><path d="M4 7a6 6 0 0 1 8 0"/><circle cx="8" cy="10" r="1" fill="#fff" stroke="none"/></svg>
    <svg width="27" height="13" viewBox="0 0 27 13"><rect x="0.5" y="0.5" width="22" height="12" rx="3.5" fill="none" stroke="#fff" opacity=".45"/><rect x="2" y="2" width="19" height="9" rx="2.2" fill="#fff"/><rect x="24" y="4.5" width="2" height="4" rx="1" fill="#fff" opacity=".45"/></svg>
  </span>`;

async function iosPrepare(page, device) {
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.addStyleTag({ content: iosChromeCss(device) });
  await page.evaluate((html) => {
    const el = document.createElement("div");
    el.id = "ios-status";
    el.innerHTML = html;
    document.body.appendChild(el);
  }, STATUS_HTML);
  await sleep(150);
}

// Visible-only tap (pages behind the tab bar stay mounted but hidden).
async function tap(page, selector, text, exact = false) {
  const ok = await page.evaluate(
    (selector, text, exact) => {
      const el = [...document.querySelectorAll(selector)].find((e) => {
        if (e.offsetParent === null && getComputedStyle(e).position !== "fixed") return false;
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
  await sleep(200);
}

async function typeField(page, selector, index, value) {
  const inputs = await page.$$(selector);
  await inputs[index].click();
  await page.keyboard.down("Control");
  await page.keyboard.press("a");
  await page.keyboard.up("Control");
  await page.keyboard.type(String(value));
}

const fields = (device) => (device === "iphone" ? ".vt-sidebar input" : "aside input");

async function iosFillConnection(page, device, ip = "10.20.4.21", deviceId = 2001) {
  await typeField(page, fields(device), 0, ip);
  await typeField(page, fields(device), 2, deviceId);
  await page.evaluate(() => document.activeElement?.blur());
  await sleep(150);
}

async function iosConnect(page, device) {
  await iosFillConnection(page, device);
  await tap(page, device === "iphone" ? ".vt-sidebar button" : "aside button", "Connect", true);
  await page.waitForFunction(
    () => document.querySelector('.bx-tab[aria-selected="true"]')?.textContent.includes("Objects") || [...document.querySelectorAll("aside button")].some((b) => b.textContent.trim().startsWith("Alarms")),
    { timeout: 5000 },
  );
  await sleep(200);
}

async function iosBrowse(page, group = "Analog Input") {
  await tap(page, "button", "Browse objects", true);
  await page.waitForSelector(".ob-group", { timeout: 5000 });
  await sleep(150);
  await tap(page, ".ob-group", group);
}

async function iosSelect(page, name) {
  const ok = await page.evaluate((name) => {
    const row = [...document.querySelectorAll(".ob-row")].find((r) => r.querySelector("span")?.textContent === name);
    if (!row) return false;
    row.click();
    return true;
  }, name);
  if (!ok) throw new Error("no object row " + name);
  await sleep(300);
}

export const iosScenes = [
  {
    id: "connect",
    devices: ["iphone"],
    headline: "Connect by <em>IP address</em>",
    sub: "Enter a controller's IP address, port and device ID and connect over BACnet/IP.",
    init: iosInit,
    setup: async (page, device) => {
      await iosPrepare(page, device);
      await iosFillConnection(page, device);
    },
  },
  {
    id: "browse",
    devices: ["iphone"],
    headline: "Browse every <em>object</em>",
    sub: "Objects are grouped by type. Tap one to read its properties.",
    init: iosInit,
    setup: async (page, device) => {
      await iosPrepare(page, device);
      await iosConnect(page, device);
      await iosBrowse(page);
    },
  },
  {
    id: "properties",
    headline: "See every <em>property</em>",
    sub: "Present value, units, limits and event state for the selected point.",
    init: iosInit,
    setup: async (page, device) => {
      await iosPrepare(page, device);
      await iosConnect(page, device);
      await iosBrowse(page);
      await iosSelect(page, "SA-TEMP");
    },
  },
  {
    id: "trend",
    headline: "Trend points as they <em>change</em>",
    sub: "Subscribe to COV notifications and watch several points on one live chart.",
    init: iosInit,
    setup: async (page, device) => {
      await iosPrepare(page, device);
      await iosConnect(page, device);
      await iosBrowse(page);
      if (device === "ipad") await iosSelect(page, "RA-CO2");
      await addTrend(page, ["SA-TEMP", "RA-TEMP", "MA-TEMP", "OA-TEMP"]);
      if (device === "iphone") await tap(page, ".bx-tab", "Trend");
      await feedCov(page, 4, 96);
    },
  },
  {
    id: "alarms",
    devices: ["ipad"],
    headline: "Check alarms, then <em>acknowledge</em>",
    sub: "Read the device's alarm summary, open each object's event timestamps and acknowledge with your name.",
    init: iosInit,
    setup: async (page, device) => {
      await iosPrepare(page, device);
      await iosConnect(page, device);
      await tap(page, "aside button", "Alarms");
      await tap(page, "main button", "Refresh");
      await page.waitForSelector("main .mono", { timeout: 5000 });
      await tap(page, "main button", "Events");
      await page.waitForFunction(() => document.body.textContent.includes("Event information"), { timeout: 5000 });
      await sleep(300);
    },
  },
  {
    id: "write",
    devices: ["ipad"],
    headline: "Write <em>setpoints</em> and commands",
    sub: "Edit a writable value, pick the data type and send it to the controller.",
    init: iosInit,
    setup: async (page, device) => {
      await iosPrepare(page, device);
      await iosConnect(page, device);
      await iosBrowse(page, "Analog Value");
      await iosSelect(page, "SA-TEMP-SP");
      await page.evaluate(() => document.querySelector('button[aria-label="Write presentValue"]').click());
      await page.waitForSelector(".vt-dialog", { timeout: 3000 });
      await sleep(300);
    },
  },
  {
    id: "simulator",
    headline: "Test without <em>any hardware</em>",
    sub: "Run a virtual air handling unit on your device, then connect to it, browse it and write its setpoints.",
    init: iosInit,
    setup: async (page, device) => {
      await iosPrepare(page, device);
      await page.select(device === "iphone" ? ".vt-sidebar select" : "aside select", "ahu");
      await sleep(150);
      await tap(page, device === "iphone" ? ".vt-sidebar button" : "aside button", "Start simulation");
      await sleep(300);
      await page.evaluate(() => window.__simTick(40));
      await sleep(300);
    },
  },
];


// ── Voltrus Connect website screenshots (named scenes only, see ../connect-docs.mjs) ──
import { connectDocScenes, connectDocIosScenes } from "../connect-docs.mjs";

const watch = (list) => list.map(([type, instance, name, source]) => ({ type, instance, name, source }));
const connectProfiles = [
  { name: "Chiller plant", shared: true, device: ["10.20.4.30", 3010], watch: watch([["analogInput", 1, "CHW-SUPPLY", "cov"], ["analogInput", 2, "CHW-RETURN", "cov"], ["analogValue", 1, "CHW-SP", "poll"]]) },
  { name: "Lobby VAV", shared: false, device: ["10.20.5.12", 4101], watch: watch([["analogInput", 1, "ZN-TEMP", "cov"], ["analogOutput", 1, "DMPR-CMD", "poll"]]) },
  { name: "Boiler room", shared: false, device: ["10.20.7.3", 8015], watch: watch([["analogInput", 1, "HW-SUPPLY", "cov"], ["binaryInput", 1, "BLR-STATUS", "poll"]]) },
].map((p) => ({ name: p.name, shared: p.shared, data: { device: { ip: p.device[0], port: 47808, device_id: p.device[1] }, watch: p.watch } }));

const connectLocal = JSON.stringify(connectProfiles.map((p) => ({ key: p.name, name: p.name, data: p.data })));

scenes.push(
  ...connectDocScenes({
    app: "bacnet-explorer",
    pre: `localStorage.setItem("bacnet-explorer-profiles", ${JSON.stringify(connectLocal)});`,
    serverProfiles: connectProfiles,
    async prepare(page) {
      await connect(page);
      await browse(page);
      await expand(page, "Analog Input");
      await selectObject(page, "SA-TEMP");
      await addTrend(page, ["SA-TEMP", "RA-TEMP", "MA-TEMP"]);
      await feedCov(page, 3, 40);
    },
    async saveFlow(page, h) {
      // The device is connected and its points are on the trend: save it, and Connect carries it to the account.
      await h.openDialog(page);
      await h.waitText(page, "Last synced");
      await h.clickTab(page, "Profiles");
      await page.type('input[placeholder="Profile name"]', "AHU-01 roof plant");
      await h.clickText(page, ".vt-dialog button", "Save");
      await h.waitText(page, "AHU-01 roof plant");
      await h.sleep(1200);
    },
  }),
);

iosScenes.push(
  ...connectDocIosScenes({
    app: "bacnet-explorer",
    init: iosInit,
    prepare: iosPrepare,
    serverProfiles: connectProfiles,
  }),
);
scenes.push(connectReviewScene("bacnet-explorer", "com.joyodigitama.bacnetexplorer"));
scenes.push(connectReviewScene("bacnet-explorer", "com.joyodigitama.bacnetexplorer", { signedIn: false }));
