// App Store scenes for Modbus Poll. Rendered by marketing/screenshots/render.mjs.
//
// The mock stands in for the Rust side of the app. Command names, arguments and return shapes follow
// apps/modbus-poll/src-tauri/src (master.rs, slave.rs, simulation.rs, system.rs) and crates/mt-modbus:
//   read_tags -> ModbusValue[] ({ Float64: n } per tag), traffic-log events (TX "count=N", RX first 8 words as
//   ["0x4366", ...]), slave_read_registers -> u16[], sim_* -> f64[], scan_bus -> { unit_id, value }[].
// The meter model is a Schneider PM5560: three phase voltages/currents, powers, PF, frequency, THD, temperature
// as float32 from register 2999, energy counters as float64 from 3203, status inputs as discrete bits 0..3.

import { pickOption } from "../pick.mjs";

function installMock() {
  const T0 = Date.now();
  const callbacks = new Map();
  let nextCallback = 1;
  const listeners = [];
  let nextListener = 1;

  function emit(event, payload) {
    for (const l of listeners) {
      if (l.event !== event) continue;
      const cb = callbacks.get(l.handler);
      if (cb) cb({ event, id: l.id, payload });
    }
  }

  // ── PM5560 model ──────────────────────────────────────────────────────────────────────────────
  const rnd = (k, t) => {
    const r = Math.sin(t * 12.9898 + k * 78.233) * 43758.5453;
    return r - Math.floor(r) - 0.5;
  };
  const smooth = (x) => {
    const c = Math.min(1, Math.max(0, x));
    return c * c * (3 - 2 * c);
  };

  /** Meter values at `t` seconds relative to page load (negative = history). */
  function meter(t) {
    // Slow breathing of the load plus a chiller stage switching on about a minute before page load.
    const step = smooth((t + 62) / 7);
    const load = 1 + 0.17 * step + 0.012 * Math.sin(t / 11) + 0.006 * Math.sin(t / 3.1);
    const i1 = 142.5 * load + 1.1 * rnd(1, t) + 0.8 * Math.sin(t / 5.3 + 1);
    const i2 = 138.7 * load + 1.1 * rnd(2, t) + 0.8 * Math.sin(t / 4.7 + 2);
    const i3 = 145.3 * load + 1.1 * rnd(3, t) + 0.8 * Math.sin(t / 6.1 + 3);
    const sag = (load - 1) * 9;
    const v1 = 230.2 - sag + 0.7 * Math.sin(t / 21) + 0.18 * rnd(4, t);
    const v2 = 229.8 - sag + 0.7 * Math.sin(t / 19 + 1) + 0.18 * rnd(5, t);
    const v3 = 230.5 - sag + 0.7 * Math.sin(t / 23 + 2) + 0.18 * rnd(6, t);
    const s = v1 * i1 + v2 * i2 + v3 * i3;
    const pf = 0.962 + 0.006 * Math.sin(t / 17) + 0.002 * rnd(7, t);
    const p = s * pf;
    const q = Math.sqrt(Math.max(0, s * s - p * p));
    return {
      v1, v2, v3, i1, i2, i3,
      p: Math.round(p), q: Math.round(q), s: Math.round(s),
      pf, hz: 50.0 + 0.012 * Math.sin(t / 7) + 0.008 * rnd(8, t),
      thdv: 1.8 + 0.12 * Math.sin(t / 13) + 0.05 * rnd(9, t),
      thdi: 6.2 + 0.5 * Math.sin(t / 9) - (step * 0.8) + 0.12 * rnd(10, t),
      temp: 38.5 + 0.2 * Math.sin(t / 40) + 0.03 * rnd(11, t),
      eImport: 1587329 + Math.floor((t + (Date.now() - T0) / 1000) / 38),
      eExport: 2451,
    };
  }

  const f32words = (v) => {
    const dv = new DataView(new ArrayBuffer(4));
    dv.setFloat32(0, v);
    return [dv.getUint16(0), dv.getUint16(2)];
  };
  const f64words = (v) => {
    const dv = new DataView(new ArrayBuffer(8));
    dv.setFloat64(0, v);
    return [dv.getUint16(0), dv.getUint16(2), dv.getUint16(4), dv.getUint16(6)];
  };

  /** Holding registers of the meter at `now`. */
  function holdingWords(from, count) {
    const m = meter((Date.now() - T0) / 1000);
    const f32 = [m.v1, m.v2, m.v3, m.i1, m.i2, m.i3, m.p, m.q, m.s, m.pf, m.hz, m.thdv, m.thdi, m.temp];
    const map = new Map();
    f32.forEach((v, k) => f32words(v).forEach((w, j) => map.set(2999 + 2 * k + j, w)));
    f64words(m.eImport).forEach((w, j) => map.set(3203 + j, w));
    f64words(m.eExport).forEach((w, j) => map.set(3207 + j, w));
    return Array.from({ length: count }, (_, j) => map.get(from + j) ?? 0);
  }
  const DISCRETE = [1, 0, 1, 0, 0, 0, 0, 0];

  function readWords(area, from, count) {
    if (area === "holding" || area === "input") return holdingWords(from, count);
    return Array.from({ length: count }, (_, j) => (DISCRETE[from + j] ?? 0));
  }

  function parseWords(words, type) {
    const dv = new DataView(new ArrayBuffer(8));
    words.forEach((w, k) => dv.setUint16(k * 2, w));
    switch (type) {
      case "float32": return dv.getFloat32(0);
      case "float64": return dv.getFloat64(0);
      case "int32": return dv.getInt32(0);
      case "int16": return dv.getInt16(0);
      default: return words[0];
    }
  }

  // ── Master ──────────────────────────────────────────────────────────────────────────────────
  const master = { connected: false, rtu: null, unit: 1 };
  const FC = { holding: 3, input: 4, coil: 1, discrete: 2 };
  const hex4 = (w) => "0x" + w.toString(16).toUpperCase().padStart(4, "0");

  /** Seconds the wire takes for one request/response pair. */
  function wireMs(words, bits) {
    if (!master.rtu) return 2 + Math.random() * 1.5;
    const respBytes = 5 + (bits ? Math.ceil(words / 8) : words * 2);
    const bytes = 8 + respBytes;
    return (bytes * 11 * 1000) / master.rtu.baud + 8 + Math.random() * 3;
  }

  function planGroups(tags) {
    const groups = [];
    let i = 0;
    while (i < tags.length) {
      const start = tags[i].address;
      let end = start + tags[i].quantity;
      let next = i + 1;
      while (next < tags.length && tags[next].register_area === tags[i].register_area && tags[next].address >= start && tags[next].address <= end) {
        end = Math.max(end, tags[next].address + tags[next].quantity);
        next++;
      }
      groups.push({ area: tags[i].register_area, start, end, first: i, last: next });
      i = next;
    }
    return groups;
  }

  async function readTags(tags, conn) {
    if (!master.connected) throw "Not connected";
    const groups = planGroups(tags);
    const results = [];
    // Frame timestamps follow the wire time, not the (software-rendered) browser's timers.
    let clock = Date.now();
    const frames = [];
    for (const g of groups) {
      const total = g.end - g.start;
      const fc = FC[g.area];
      const bit = g.area === "coil" || g.area === "discrete";
      frames.push({ conn, timestamp: clock, direction: "TX", function_code: fc, address: g.start, data: `count=${total}`, length: total });
      clock += Math.round(wireMs(total, bit));
      const words = readWords(g.area, g.start, total);
      frames.push({
        conn, timestamp: clock, direction: "RX", function_code: fc, address: g.start,
        data: JSON.stringify(words.slice(0, 8).map(hex4)).replace(/,/g, ", "), length: words.length,
      });
      clock += 1;
      for (let k = g.first; k < g.last; k++) {
        const t = tags[k];
        const off = t.address - g.start;
        const raw = bit ? words[off] : parseWords(words.slice(off, off + t.quantity), t.type);
        results.push({ Float64: raw * t.scale });
      }
    }
    await new Promise((r) => setTimeout(r, Math.max(1, clock - Date.now())));
    // Frames reach the UI as the read returns, so rendering them does not count towards the measured round trip.
    setTimeout(() => frames.forEach((f) => emit("traffic-log", f)), 0);
    return results;
  }

  // ── Slave + simulation ──────────────────────────────────────────────────────────────────────
  const slave = { running: false, regs: [], profile: null, values: [], locks: new Set(), timer: null, logTimer: null };

  function writeProfile() {
    const p = slave.profile;
    let off = p.baseAddress;
    p.registers.forEach((def, i) => {
      const words = def.isCounter ? f64words(slave.values[i]) : f32words(slave.values[i]);
      words.forEach((w, j) => { if (off + j < slave.regs.length) slave.regs[off + j] = w; });
      off += words.length;
    });
  }

  function simTick(dt) {
    const p = slave.profile;
    p.registers.forEach((def, i) => {
      if (slave.locks.has(i)) return;
      let v = slave.values[i];
      if (def.isCounter) v += def.counterRate * dt;
      else {
        v += (Math.random() - 0.5) * 2 * def.drift + (def.initial - v) * 0.06;
        v = Math.min(def.max, Math.max(def.min, v));
      }
      slave.values[i] = v;
    });
    writeProfile();
  }

  function startSlaveTraffic() {
    clearInterval(slave.logTimer);
    let n = 0;
    slave.logTimer = setInterval(() => {
      if (!slave.running) return;
      n++;
      const base = slave.profile ? slave.profile.baseAddress : 0;
      const words = slave.profile ? slave.profile.registers.reduce((a, r) => a + (r.isCounter ? 4 : 2), 0) : 10;
      emit("slave-log", {
        timestamp: Date.now(), function_code: 3, address: base, quantity: words,
        message: `Read holding regs ${base}..${base + words} (${words} words)`,
      });
    }, 1300);
  }

  // ── Tauri runtime ───────────────────────────────────────────────────────────────────────────
  const cfg = () => window.__MOCK_CFG || {};

  const commands = {
    store_status: () => ({ store: false, pro: true, reason: "build", price: null }),
    platform: () => cfg().platform || "macos",
    open_url: () => null,
    list_serial_ports: () => cfg().serialPorts || [],
    connect: ({ connId, config }) => {
      master.connected = true; master.rtu = null; master.unit = config.unit_id;
      emit("tcp-event", { conn: connId, timestamp: Date.now(), event_type: "connecting", detail: `${config.host}:${config.port}` });
      emit("tcp-event", { conn: connId, timestamp: Date.now(), event_type: "connect", detail: `${config.host}:${config.port} (${12 + Math.floor(Math.random() * 8)}ms)` });
      return null;
    },
    connect_rtu: ({ connId, serialConfig, unitId }) => {
      master.connected = true; master.rtu = { baud: serialConfig.baud_rate }; master.unit = unitId;
      emit("tcp-event", { conn: connId, timestamp: Date.now(), event_type: "connect", detail: `${serialConfig.port_name} @ ${serialConfig.baud_rate} baud` });
      return new Promise((r) => setTimeout(() => r(null), 250));
    },
    disconnect: ({ connId }) => { master.connected = false; emit("tcp-event", { conn: connId, timestamp: Date.now(), event_type: "disconnect", detail: "Client disconnected" }); return null; },
    set_unit_id: ({ unitId }) => { master.unit = unitId; return null; },
    read_tags: ({ connId, tags }) => readTags(tags, connId),
    write_tag: () => null,
    scan_bus: ({ connId, startId, endId }) => {
      const found = (cfg().scanResults || []).filter((r) => r.unit_id >= startId && r.unit_id <= endId);
      emit("tcp-event", { conn: connId, timestamp: Date.now(), event_type: "info", detail: `Bus scan complete: ${found.length}/${endId - startId + 1} devices found` });
      return new Promise((r) => setTimeout(() => r(found), 900));
    },
    stop_scan_bus: () => null,
    scan_network: () => cfg().networkResults || [],
    auto_detect_rtu: () => ({ baud_rate: 19200, parity: "even", stop_bits: 1, data_bits: 8 }),

    slave_start: ({ config }) => {
      slave.running = true; slave.regs = new Array(config.register_count).fill(0); slave.locks = new Set();
      emit("slave-log", { timestamp: Date.now(), function_code: 0, address: 0, quantity: 0, message: `Slave server listening on ${config.bind_addr}:${config.port} (unit ${config.unit_id}, ${config.register_count} registers)` });
      startSlaveTraffic();
      return null;
    },
    slave_stop: () => {
      slave.running = false; clearInterval(slave.timer); clearInterval(slave.logTimer); slave.timer = null;
      slave.values = []; slave.regs = [];
      return null;
    },
    slave_read_registers: ({ start, count }) => {
      if (!slave.running) throw "Slave not running";
      return slave.regs.slice(start, start + count);
    },
    slave_write_register: ({ address, value }) => { slave.regs[address] = value; return null; },
    sim_load_profile: ({ profile }) => {
      if (!slave.running) throw "Slave not running";
      clearInterval(slave.timer); slave.timer = null;
      slave.profile = profile; slave.values = profile.registers.map((r) => r.initial); slave.locks = new Set();
      writeProfile();
      return null;
    },
    sim_start: ({ profile, intervalMs }) => {
      if (!slave.running) throw "Slave not running";
      clearInterval(slave.timer);
      if (!slave.profile || slave.profile.id !== profile.id) { slave.profile = profile; slave.values = profile.registers.map((r) => r.initial); }
      simTick(intervalMs / 1000);
      slave.timer = setInterval(() => simTick(intervalMs / 1000), intervalMs);
      return null;
    },
    sim_stop: () => { clearInterval(slave.timer); slave.timer = null; return null; },
    sim_read_values: () => {
      if (!slave.profile) throw "No profile loaded";
      return slave.values.slice();
    },
    sim_lock_register: ({ registerIndex, locked }) => {
      if (registerIndex >= slave.values.length) throw `Register index ${registerIndex} out of range (${slave.values.length} registers)`;
      if (locked) slave.locks.add(registerIndex); else slave.locks.delete(registerIndex);
      return null;
    },
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
  // The shared Toolbar reserves the traffic-light inset only while the window is smaller than the screen.
  Object.defineProperty(screen, "width", { get: () => 2560 });
  Object.defineProperty(screen, "height", { get: () => 1600 });
  // Smooth scrolling lags behind in the software-rendered browser, which leaves the log panel on stale rows.
  const scrollIntoView = Element.prototype.scrollIntoView;
  Element.prototype.scrollIntoView = function (arg) {
    return scrollIntoView.call(this, arg && typeof arg === "object" ? { ...arg, behavior: "auto" } : arg);
  };
  window.__mock = { meter, T0, emit };
}

export const mock = `(${installMock.toString()})();`;

// ── Shared scene plumbing ────────────────────────────────────────────────────────────────────────

import { connectReviewScene } from "../connect-review.mjs";

const PROFILE_BLOCKS = [
  { name: "PM5560", address: 2999, quantity: 14, type: "float32", area: "holding" },
  { name: "Energy", address: 3203, quantity: 2, type: "float64", area: "holding" },
  { name: "DI", address: 0, quantity: 4, type: "uint16", area: "discrete" },
];

const profiles = [
  {
    name: "MCC-1 incomer",
    transport: "tcp",
    tcp: { host: "192.168.10.45", port: 502, unitId: 1, pollInterval: 250 },
    rtu: { port: "", baudRate: 9600, dataBits: 8, stopBits: 1, parity: "none", unitId: 1 },
    blocks: PROFILE_BLOCKS,
  },
  {
    name: "Chiller 1",
    transport: "tcp",
    tcp: { host: "192.168.10.46", port: 502, unitId: 1, pollInterval: 500 },
    rtu: { port: "", baudRate: 9600, dataBits: 8, stopBits: 1, parity: "none", unitId: 1 },
    blocks: PROFILE_BLOCKS,
  },
  {
    name: "Panel board RS-485",
    transport: "rtu",
    tcp: { host: "192.168.10.45", port: 502, unitId: 1, pollInterval: 500 },
    rtu: { port: "/dev/cu.usbserial-FT9XK2LQ", baudRate: 19200, dataBits: 8, stopBits: 1, parity: "even", unitId: 1 },
    blocks: PROFILE_BLOCKS.slice(0, 2),
  },
];

const initCommon = `
  localStorage.setItem("modbus-poll-theme", "dark");
  localStorage.setItem("modbus-poll-conn-profiles", ${JSON.stringify(JSON.stringify(profiles))});
  window.__MOCK_CFG = {
    serialPorts: [
      "/dev/cu.Bluetooth-Incoming-Port",
      "/dev/cu.usbserial-FT9XK2LQ",
      "/dev/tty.Bluetooth-Incoming-Port",
      "/dev/tty.usbserial-FT9XK2LQ",
    ],
    scanResults: [
      { unit_id: 1, value: 17254 },
      { unit_id: 2, value: 17253 },
      { unit_id: 5, value: 17251 },
    ],
  };
`;

// The frame crops the bottom of the window, so keep the app's own bottom edge (status bar) inside the visible part;
// scrollbars are overlay-style on macOS.
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

async function setStore(page, patch) {
  await page.evaluate(async (patchSrc) => {
    const { useStore } = await import("/src/store.ts");
    useStore.setState(new Function("return " + patchSrc)());
  }, patch);
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

async function chooseProfile(page, name) {
  await pickOption(page, "New profile", name);
}

/** Sizes the comm log to a whole number of rows so no half row shows under its header; the leftover is dark spacing. */
async function fitLog(page) {
  await page.evaluate(() => {
    const row = document.querySelector('[title="Click to copy"]');
    const box = row?.parentElement;
    if (!box) return;
    const h = row.getBoundingClientRect().height;
    const panel = box.parentElement;
    const available = panel.clientHeight - [...panel.children].filter((c) => c !== box).reduce((a, c) => a + c.offsetHeight, 0);
    box.style.flex = "none";
    box.style.height = `${Math.floor(available / h) * h - 1}px`;
    box.scrollTop = box.scrollHeight;
  });
}

/** Waits for the last `n` comm log polls to all report at most `maxMs`. */
async function waitCalmLog(page, maxMs, n = 5) {
  await page.waitForFunction((max, count) => {
    const rows = [...document.querySelectorAll('[title="Click to copy"]')].slice(-count);
    return rows.length === count && rows.every((r) => {
      const m = /\((\d+)ms\)/.exec(r.textContent);
      return m && Number(m[1]) <= max;
    });
  }, { timeout: 90000, polling: 50 }, maxMs, n).catch(() => {});
}

/** Waits (up to 60 s) for six consecutive polls with a round-trip time of at most `maxMs`; the software-rendered browser jitters. */
async function waitLatency(page, maxMs) {
  await page.waitForFunction((max) => {
    // Desktop: the status bar ("12 ms 30 polls"). Phone: the header's endpoint line ("MCC-1 incomer · 12 ms") and the chip's poll count.
    const m = /(\d+) ms\s*([\d,]+) polls/.exec(document.querySelector("footer")?.textContent ?? "");
    const phone = document.querySelector("[data-polls]");
    const ms = m ? Number(m[1]) : Number(/(\d+) ms/.exec(document.querySelector(".vm-header__context")?.textContent ?? "")?.[1]);
    const polls = m ? Number(m[2].replace(/,/g, "")) : Number(phone?.getAttribute("data-polls"));
    if (!Number.isFinite(ms) || !Number.isFinite(polls)) return false;
    const calm = (window.__calm ??= { last: -1, n: 0 });
    if (polls !== calm.last) {
      calm.last = polls;
      calm.n = ms <= max ? calm.n + 1 : 0;
    }
    return calm.n >= 6;
  }, { timeout: 60000, polling: 50 }, maxMs).catch(() => {});
}

async function waitPolls(page, n) {
  await page.waitForFunction((count) => {
    const m = /([\d,]+) polls/.exec(document.querySelector("footer")?.textContent ?? "");
    if (m) return Number(m[1].replace(/,/g, "")) >= count;
    return Number(document.querySelector("[data-polls]")?.getAttribute("data-polls")) >= count;
  }, { timeout: 90000 }, n);
}

async function connectTcp(page, profile, polls) {
  await chooseProfile(page, profile);
  await clickButton(page, "Connect");
  await waitPolls(page, polls);
}

/** Series seeded from the same meter model as the live reads, so the chart carries on from its history. */
async function seedTrend(page, names, spec, seconds) {
  await page.evaluate(async (names, spec, seconds) => {
    const { useStore } = await import("/src/store.ts");
    const colors = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7"];
    const now = Date.now();
    const m = new Map();
    names.forEach((name, i) => {
      const points = [];
      for (let s = seconds; s >= 1; s--) {
        const t = -s;
        points.push({ t: now - s * 1000, v: window.__mock.meter(t)[spec[i]] });
      }
      m.set(name, { name, color: colors[i % colors.length], points });
    });
    useStore.setState({ trendSeries: m });
  }, names, spec, seconds);
}

/** Scrolls the master table so the named row sits right under the sticky header. */
async function scrollTableTo(page, name) {
  await page.evaluate((n) => {
    const cell = [...document.querySelectorAll("tbody td")].find((td) => td.textContent.trim() === n);
    const row = cell.parentElement;
    let scroller = row.parentElement;
    while (scroller && !/(auto|scroll)/.test(getComputedStyle(scroller).overflowY)) scroller = scroller.parentElement;
    const head = scroller.querySelector("thead");
    scroller.scrollTop += row.getBoundingClientRect().top - scroller.getBoundingClientRect().top - head.offsetHeight;
  }, name);
}

// ── Scenes ──────────────────────────────────────────────────────────────────────────────────────

export const uiScale = 1.12;

export const scenes = [
  {
    id: "live-polling",
    headline: "See meter values <em>live</em>",
    sub: "Read float32 and float64 registers over Modbus TCP, with round-trip time in the status bar.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 1.12);
      await setStore(page, `({ leftWidth: 236, rightWidth: 236, tableHeight: 368 })`);
      await connectTcp(page, "MCC-1 incomer", 70);
      await fitLog(page);
      await waitCalmLog(page, 16);
    },
  },
  {
    id: "slave-simulator",
    headline: "Simulate a device <em>without hardware</em>",
    sub: "Run a Modbus slave from a PM5560 template: values drift, energy counts up, registers can be locked.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 0.95);
      await setStore(page, `({ mode: "slave", leftWidth: 196, rightWidth: 262, tableHeight: 362, slavePollMs: 500 })`);
      await sleep(300);
      await clickButton(page, "Start server");
      await sleep(400);
      await pickOption(page, "Select device…", "pm5560", "Schneider Electric PM5560");
      await sleep(600);
      await clickButton(page, "Simulate");
      await sleep(1500);
      await (await page.$$('button[aria-label="Lock register"]'))[9].click();
      await sleep(1500);
      await page.evaluate(() => {
        for (const el of document.querySelectorAll("table")) if (el.parentElement.scrollTop) el.parentElement.scrollTop = 0;
      });
      await fitLog(page);
      await sleep(4500);
    },
  },
  {
    id: "traffic",
    headline: "Every request and response, <em>decoded</em>",
    sub: "Each frame shows its function code, address range and register words in hex. Click a row to expand.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 1.0);
      await setStore(page, `({ leftWidth: 200, rightWidth: 236, tableHeight: 158, bottomTab: "traffic" })`);
      await chooseProfile(page, "MCC-1 incomer");
      await clickButton(page, "Connect");
      await waitPolls(page, 8);
      // A short history keeps the panel cheap to render, so the status bar shows a realistic round-trip time.
      await page.click('button[aria-label="Clear all traffic"]');
      await sleep(3500);
      // Stop following the tail, expand one response and park the view on its request.
      await page.click('input[aria-label="Auto-scroll"]');
      const rowsOfPoll = () => {
        const rows = [...document.querySelectorAll('tr[title="Click to expand frame details"]')];
        return { rows, i: rows.findIndex((r, k) => k >= 6 && r.textContent.includes("count=28")) };
      };
      await page.evaluate((src) => {
        const { rows, i } = new Function("return " + src)()();
        rows[i + 1].click();
      }, rowsOfPoll.toString());
      await sleep(300);
      await page.evaluate((src) => {
        const { rows, i } = new Function("return " + src)()();
        const scroller = rows[i].closest("div");
        const head = scroller.querySelector("thead");
        scroller.scrollTop += rows[i].getBoundingClientRect().top - scroller.getBoundingClientRect().top - head.offsetHeight;
      }, rowsOfPoll.toString());
      await waitLatency(page, 14);
    },
  },
  {
    id: "trend",
    headline: "Catch load changes <em>on a trend</em>",
    sub: "Click Chart on any row to plot it live, with 30 second, 1 minute and 5 minute windows.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 1.12);
      await setStore(page, `({ leftWidth: 236, rightWidth: 236, tableHeight: 154, trendWindow: 300 })`);
      await chooseProfile(page, "MCC-1 incomer");
      await clickButton(page, "Connect");
      await waitPolls(page, 12);
      await seedTrend(page, ["PM5560_3005", "PM5560_3007", "PM5560_3009"], ["i1", "i2", "i3"], 150);
      // The chart is created when the Trend panel mounts with series present.
      await setStore(page, `({ bottomTab: "trend" })`);
      await scrollTableTo(page, "PM5560_3005");
      await sleep(4000);
      await waitLatency(page, 14);
    },
  },
  {
    id: "rtu-bus-scan",
    headline: "Poll RS-485 <em>from your Mac</em>",
    sub: "Pick a USB serial adapter, set baud and parity, then scan unit IDs 1 to 16 for devices.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 1.0);
      await setStore(page, `({ leftWidth: 290, rightWidth: 236, tableHeight: 332 })`);
      await chooseProfile(page, "Panel board RS-485");
      await sleep(600);
      await setStore(page, `({ rtuTimeout: 1000, rtuPollInterval: 500, busScanAddr: 2999, busScanStart: 1, busScanEnd: 16, showBusScanner: true })`);
      await clickButton(page, "Connect RTU");
      await waitPolls(page, 8);
      await clickButton(page, "Scan bus");
      await page.waitForFunction(() => document.body.textContent.includes("device(s) found"), { timeout: 30000 });
      await waitPolls(page, 30);
      await page.click('tr[title="Poll this unit ID"]');
      await sleep(300);
      await clickButton(page, "Info");
      await fitLog(page);
      // Sidebar: start at the Port field so the serial settings, scan controls and results fit together.
      await page.evaluate(() => {
        const aside = document.querySelector("aside");
        const label = [...aside.querySelectorAll("label,span,div")].find((e) => e.children.length === 0 && e.textContent.trim() === "Port");
        aside.scrollTop = label.getBoundingClientRect().top - aside.getBoundingClientRect().top + aside.scrollTop - 8;
      });
    },
  },
];

// ── iPhone / iPad scenes (render.mjs --device=iphone|ipad) ──────────────────────────────────────
// iPhone (390 CSS px wide) uses the app's mobile layout (isMobile); iPad (834 px) is above the 768 px breakpoint and
// uses the desktop layout. iOS has no serial ports, so only the TCP profiles are seeded and nothing shows RTU.

const iosInit =
  initCommon.replace(
    JSON.stringify(JSON.stringify(profiles)),
    JSON.stringify(JSON.stringify(profiles.filter((p) => p.transport === "tcp"))),
  ) + `window.__MOCK_CFG.platform = "ios";`;

const iosCss = `*::-webkit-scrollbar { display: none; } * { scrollbar-width: none; }`;

async function iosPrepare(page) {
  await page.addStyleTag({ content: iosCss });
}

/** Opens a phone tab; tabs past the fifth (Log, Traffic) sit in the More sheet. */
async function mobileTab(page, label) {
  const click = (selector, t) =>
    page.evaluate((selector, t) => {
      const b = [...document.querySelectorAll(selector)].find((el) => el.textContent.trim().startsWith(t));
      if (!b) return false;
      b.click();
      return true;
    }, selector, t);
  if (!(await click("nav button", label))) {
    if (!(await click("nav button", "More"))) throw new Error(`tab "${label}" not found`);
    await sleep(500);
    if (!(await click(".vt-dialog [role=button]", label))) throw new Error(`tab "${label}" not found in More`);
  }
  await sleep(500);
}

export const iosScenes = [
  {
    id: "live-polling",
    headline: "See meter values <em>live</em>",
    sub: "Read float32 and float64 registers over Modbus TCP, right from your iPhone or iPad.",
    init: iosInit,
    settleMs: 400,
    async setup(page, device) {
      await iosPrepare(page);
      if (device === "iphone") {
        await connectTcp(page, "MCC-1 incomer", 70);
        await mobileTab(page, "Data");
      } else {
        await setStore(page, `({ leftWidth: 236, rightWidth: 200, tableHeight: 560 })`);
        await connectTcp(page, "MCC-1 incomer", 70);
      }
      await waitLatency(page, 14);
    },
  },
  {
    id: "connect",
    headline: "Connect to a PLC or meter <em>in seconds</em>",
    sub: "Save a profile with host, port and unit ID, then reconnect with one tap.",
    init: iosInit,
    settleMs: 400,
    async setup(page, device) {
      await iosPrepare(page);
      if (device === "iphone") {
        await chooseProfile(page, "MCC-1 incomer");
        await sleep(1200);
      } else {
        await setStore(page, `({ leftWidth: 300, rightWidth: 230, tableHeight: 420 })`);
        await connectTcp(page, "Chiller 1", 30);
      }
    },
  },
  {
    id: "slave-simulator",
    headline: "Simulate a device <em>without hardware</em>",
    sub: "Run a Modbus TCP slave from a PM5560 template. Values drift and energy counts up.",
    init: iosInit,
    settleMs: 400,
    async setup(page, device) {
      await iosPrepare(page);
      await setStore(page, device === "iphone" ? `({ mode: "slave", slavePollMs: 500 })` : `({ mode: "slave", leftWidth: 230, rightWidth: 250, tableHeight: 560, slavePollMs: 500 })`);
      await sleep(300);
      await clickButton(page, "Start server");
      await sleep(400);
      if (device === "iphone") await mobileTab(page, "Template");
      await pickOption(page, "Select device…", "pm5560", "Schneider Electric PM5560");
      await sleep(600);
      await clickButton(page, "Simulate");
      await sleep(1500);
      if (device === "iphone") await mobileTab(page, "Data");
      await sleep(3500);
    },
  },
  {
    id: "log",
    devices: ["iphone"],
    headline: "Follow every poll <em>as it happens</em>",
    sub: "The comm log shows each read with its round-trip time.",
    init: iosInit,
    settleMs: 400,
    async setup(page) {
      await iosPrepare(page);
      await chooseProfile(page, "MCC-1 incomer");
      await clickButton(page, "Connect");
      await waitPolls(page, 12);
      await mobileTab(page, "Log");
      await sleep(2500);
    },
  },
  {
    id: "traffic",
    devices: ["ipad"],
    headline: "Every request and response, <em>decoded</em>",
    sub: "Each frame shows its function code, address range and register words in hex.",
    init: iosInit,
    settleMs: 400,
    async setup(page, device) {
      await iosPrepare(page);
      if (device === "ipad") await setStore(page, `({ leftWidth: 220, rightWidth: 200, tableHeight: 300, bottomTab: "traffic" })`);
      await chooseProfile(page, "MCC-1 incomer");
      await clickButton(page, "Connect");
      await waitPolls(page, 8);
      if (device === "iphone") await mobileTab(page, "Traffic");
      await page.click('button[aria-label="Clear all traffic"]');
      await sleep(3500);
      await page.evaluate(() => {
        const rows = [...document.querySelectorAll('[title="Click to expand frame details"]')];
        rows.find((r, k) => k >= 3 && r.textContent.includes("RX"))?.click();
      });
      await sleep(400);
    },
  },
  {
    id: "trend",
    headline: "Catch load changes <em>on a trend</em>",
    sub: "Plot any register live with 30 second, 1 minute and 5 minute windows.",
    init: iosInit,
    settleMs: 400,
    async setup(page, device) {
      await iosPrepare(page);
      if (device === "ipad") await setStore(page, `({ leftWidth: 220, rightWidth: 200, tableHeight: 300, trendWindow: 300 })`);
      else await setStore(page, `({ trendWindow: 300 })`);
      await chooseProfile(page, "MCC-1 incomer");
      await clickButton(page, "Connect");
      await waitPolls(page, 12);
      await seedTrend(page, ["PM5560_3005", "PM5560_3007", "PM5560_3009"], ["i1", "i2", "i3"], 150);
      await setStore(page, device === "iphone" ? `({ mobileTab: "trend" })` : `({ bottomTab: "trend" })`);
      await sleep(4000);
    },
  },
];


// ── Voltrus Connect website screenshots (named scenes only, see ../connect-docs.mjs) ──
import { connectDocScenes, connectDocIosScenes } from "../connect-docs.mjs";

const connectProfiles = profiles.map((p) => ({
  name: p.name,
  shared: p.name === "Chiller 1",
  data: { transport: p.transport, tcp: p.tcp, rtu: p.rtu, blocks: p.blocks },
}));

scenes.push(
  ...connectDocScenes({
    app: "modbus-poll",
    pre: initCommon,
    serverProfiles: connectProfiles,
    async prepare(page) {
      await connectTcp(page, "MCC-1 incomer", 40);
      await waitLatency(page, 16);
    },
    async saveFlow(page, h) {
      // A live connection nobody has saved under this name yet: name it, and Connect carries it to the account.
      await clickButton(page, "Save as new");
      await h.sleep(300);
      await clickButton(page, "Save as profile…");
      await h.sleep(300);
      await page.type('input[placeholder="Profile name"]', "Boiler meter");
      await page.evaluate(() => {
        const input = document.querySelector('input[placeholder="Profile name"]');
        let el = input;
        while (el && !/(auto|scroll)/.test(getComputedStyle(el).overflowY)) el = el.parentElement;
        if (el) el.scrollTop = el.scrollHeight;
      });
      await h.sleep(400);
    },
  }),
);

iosScenes.push(
  ...connectDocIosScenes({
    app: "modbus-poll",
    init: iosInit + `localStorage.setItem("modbus-poll-conn-profiles", "[]");`,
    prepare: iosPrepare,
    serverProfiles: connectProfiles.filter((p) => p.data.transport === "tcp"),
  }),
);
scenes.push(connectReviewScene("modbus-poll", "com.joyodigitama.modbuspoll"));
scenes.push(connectReviewScene("modbus-poll", "com.joyodigitama.modbuspoll", { signedIn: false }));
