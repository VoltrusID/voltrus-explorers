// App Store scenes for DNP3 Explorer. Rendered by marketing/screenshots/render.mjs.
//
// The mock stands in for the Rust side (apps/dnp3-explorer/src-tauri/src/lib.rs):
//   connect, read_tags -> { point, value }[], event_scan, time_sync/restarts/freeze, write_analog, write_tag,
//   outstation_* (read_points -> [analogs, binaries, counters]), sim_* (sim_read_values -> f64[]), store_status.
// The outstation is a 13.8 kV feeder RTU: line voltages, phase currents, frequency, MW, breaker status, kWh counter.

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

  const rnd = (k, t) => {
    const r = Math.sin(t * 12.9898 + k * 78.233) * 43758.5453;
    return r - Math.floor(r) - 0.5;
  };

  /** Feeder RTU points keyed "group:index" at `t` seconds. */
  function rtu(g, i, t) {
    const load = 1 + 0.04 * Math.sin(t / 9) + 0.015 * Math.sin(t / 2.7);
    if (g === 30) {
      switch (i) {
        case 0: return 13.82 + 0.03 * Math.sin(t / 21) + 0.006 * rnd(1, t);
        case 1: return 13.76 + 0.03 * Math.sin(t / 19 + 1) + 0.006 * rnd(2, t);
        case 2: return 13.89 + 0.03 * Math.sin(t / 23 + 2) + 0.006 * rnd(3, t);
        case 3: return 142.3 * load + 0.6 * rnd(4, t);
        case 4: return 138.7 * load + 0.6 * rnd(5, t);
        case 5: return 145.1 * load + 0.6 * rnd(6, t);
        case 6: return 50.02 + 0.015 * Math.sin(t / 7) + 0.004 * rnd(7, t);
        case 7: return 3.42 * load + 0.01 * rnd(8, t);
        case 8: return 0.85 * load + 0.01 * rnd(9, t);
        case 9: return 0.97 + 0.003 * Math.sin(t / 13);
        default: return 0;
      }
    }
    if (g === 1 || g === 10) return i === 0 || i === 1 ? 1 : 0;
    if (g === 20) return 481250 + i * 3100 + Math.floor((t + (Date.now() - T0) / 1000) * 0.9);
    return 0;
  }

  const sec = () => (Date.now() - T0) / 1000;
  const state = { connected: false, os: false, osCfg: null, simProfile: null, simTimer: null, simVals: [] };
  const ts = () => new Date().toTimeString().slice(0, 8);

  function simTick() {
    const t = sec();
    state.simVals = state.simProfile.points.map((p) => {
      if (p.isBool) return p.base;
      if (p.group === 20) return (state.simVals[state.simProfile.points.indexOf(p)] ?? p.base) + p.variance;
      const v = p.base + p.variance * Math.sin(t * 2 * Math.PI / Math.max(1, p.periodMs / 1.2)) + p.noise * rnd(p.index + 20, t);
      return Math.min(p.max ?? 1e9, Math.max(p.min ?? -1e9, v));
    });
  }

  const commands = {
    store_status: () => ({ store: false, pro: true, reason: "build", price: null }),
    open_url: () => null,
    connect: async () => { await new Promise((r) => setTimeout(r, 120)); state.connected = true; return null; },
    disconnect: () => { state.connected = false; return null; },
    read_tags: async ({ tags }) => {
      if (!state.connected) throw "Not connected";
      await new Promise((r) => setTimeout(r, 6 + Math.random() * 4));
      const t = sec();
      return tags.map((point) => ({ point, value: rtu(point.group, point.index, t) }));
    },
    event_scan: async ({ class: c }) => {
      await new Promise((r) => setTimeout(r, 40));
      return Array.from({ length: c === 1 ? 4 : c === 2 ? 7 : 2 }, (_, k) => ({ k }));
    },
    time_sync: () => null,
    cold_restart: () => null,
    warm_restart: () => null,
    freeze_counters: () => null,
    write_analog: () => null,
    write_tag: () => null,
    outstation_start: ({ config }) => { state.os = true; state.osCfg = config; return null; },
    outstation_stop: () => { state.os = false; return null; },
    outstation_read_points: () => {
      const t = sec();
      const c = state.osCfg || {};
      const analogs = Array.from({ length: c.analog_count ?? 10 }, (_, i) => {
        if (state.simProfile) {
          const idx = state.simProfile.points.findIndex((p) => !p.isBool && p.group !== 20 && p.index === i);
          if (idx >= 0) return state.simVals[idx] ?? 0;
        }
        return rtu(30, i, t);
      });
      const binaries = Array.from({ length: c.binary_count ?? 10 }, (_, i) => {
        if (state.simProfile) {
          const idx = state.simProfile.points.findIndex((p) => p.isBool && p.index === i);
          if (idx >= 0) return (state.simVals[idx] ?? 0) >= 0.5;
        }
        return i < 2;
      });
      const counters = Array.from({ length: c.counter_count ?? 4 }, (_, i) => {
        if (state.simProfile) {
          const idx = state.simProfile.points.findIndex((p) => p.group === 20 && p.index === i);
          if (idx >= 0) return Math.round(state.simVals[idx] ?? 0);
        }
        return Math.round(rtu(20, i, t));
      });
      return [analogs, binaries, counters];
    },
    outstation_read_events: () => {
      if (!state.simProfile) return [];
      const n = Math.min(8, Math.floor(sec() / 1.5));
      return Array.from({ length: n }, (_, k) => ({
        class: 1 + (k % 3), group: k % 4 === 3 ? 1 : 30, index: k % 4 === 3 ? 0 : [3, 4, 7, 5][k % 4],
        value: k % 4 === 3 ? 1 : 142 + k * 0.7, timestamp: Date.now() - (n - k) * 1500, sequence: k + 1,
      }));
    },
    outstation_write_analog: () => null,
    outstation_write_binary: () => null,
    sim_load_profile: ({ profile }) => { state.simProfile = profile; state.simVals = profile.points.map((p) => p.base); return null; },
    sim_start: ({ profile, intervalMs }) => {
      if (!state.simProfile || state.simProfile.id !== profile.id) { state.simProfile = profile; state.simVals = profile.points.map((p) => p.base); }
      clearInterval(state.simTimer);
      simTick();
      state.simTimer = setInterval(simTick, intervalMs);
      return null;
    },
    sim_stop: () => { clearInterval(state.simTimer); return null; },
    sim_read_values: () => { if (!state.simProfile) throw "No profile loaded"; return state.simVals.slice(); },
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
    transformCallback(fn) { const id = nextCallback++; callbacks.set(id, fn); return id; },
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
  window.__mock = { emit, ts };
}

export const mock = `(${installMock.toString()})();`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const POINTS = [
  { name: "Voltage_AB", group: 30, variation: 4, index: 0 },
  { name: "Voltage_BC", group: 30, variation: 4, index: 1 },
  { name: "Voltage_CA", group: 30, variation: 4, index: 2 },
  { name: "Current_A", group: 30, variation: 4, index: 3 },
  { name: "Current_B", group: 30, variation: 4, index: 4 },
  { name: "Current_C", group: 30, variation: 4, index: 5 },
  { name: "Frequency", group: 30, variation: 4, index: 6 },
  { name: "MW", group: 30, variation: 4, index: 7 },
  { name: "Breaker_52_1", group: 1, variation: 2, index: 0 },
  { name: "Energy_kWh", group: 20, variation: 1, index: 0 },
];

const PROFILES = [
  { name: "Feeder 12 RTU", host: "10.20.30.41", port: 20000, masterAddress: 1, outstationAddress: 10 },
];

const init = `
  localStorage.setItem("dnp3-explorer-points", ${JSON.stringify(JSON.stringify(POINTS))});
  localStorage.setItem("dnp3-explorer-conn-profiles", ${JSON.stringify(JSON.stringify(PROFILES))});
`;

async function clickButton(page, text) {
  const ok = await page.evaluate((t) => {
    const b = [...document.querySelectorAll("button")].find((el) => el.textContent.trim() === t && !el.disabled);
    if (!b) return false;
    b.click();
    return true;
  }, text);
  if (!ok) throw new Error(`button "${text}" not found`);
}

/** Drags the table splitter down so all ten points show above the log. */
async function growTable(page, dy) {
  const box = await page.evaluate(() => {
    const h = [...document.querySelectorAll("div")].find((d) => d.style.cursor === "row-resize");
    const b = h.getBoundingClientRect();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  });
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  await page.mouse.move(box.x, box.y + dy, { steps: 6 });
  await page.mouse.up();
  await sleep(300);
}

async function scrollSidebarTo(page, title) {
  await page.evaluate((t) => {
    const aside = document.querySelector("aside");
    const h = [...aside.querySelectorAll("*")].find((el) => el.children.length === 0 && el.textContent.trim() === t);
    aside.scrollTop += h.getBoundingClientRect().top - aside.getBoundingClientRect().top - 6;
  }, title);
  await sleep(300);
}

async function connect(page) {
  await page.evaluate(() => {
    const s = [...document.querySelectorAll("select")].find((el) => [...el.options].some((o) => o.textContent === "New profile"));
    s.setAttribute("data-profile-select", "1");
  });
  await page.select("[data-profile-select]", "Feeder 12 RTU");
  await sleep(300);
  await clickButton(page, "Connect");
  await sleep(2500);
}

export const scenes = [
  {
    id: "live-points",
    headline: "Watch your outstation <em>live</em>",
    sub: "Build your own point list, then poll analog, binary and counter points from any DNP3 outstation.",
    init,
    async setup(page) {
      await connect(page);
      await growTable(page, 40);
      await scrollSidebarTo(page, "Points");
      await sleep(2000);
    },
  },
  {
    id: "controls",
    headline: "Operate controls <em>with confidence</em>",
    sub: "Open a point to send a control relay or analog output command and see the response in the log.",
    init,
    async setup(page) {
      await connect(page);
      await growTable(page, 40);
      await page.evaluate(() => {
        const row = [...document.querySelectorAll("tr")].find((r) => r.textContent.includes("Breaker_52_1"));
        row.click();
      });
      await sleep(800);
    },
  },
  {
    id: "class-scans",
    headline: "Scan events by <em>class</em>",
    sub: "Run class 1, 2 and 3 event scans, sync the clock and send restarts, with every step in the log.",
    init,
    async setup(page) {
      await connect(page);
      await clickButton(page, "Class 1");
      await sleep(500);
      await clickButton(page, "Class 2");
      await sleep(500);
      await clickButton(page, "Time sync");
      await sleep(600);
      await growTable(page, 40);
      await scrollSidebarTo(page, "Class polling");
      await sleep(800);
    },
  },
  {
    id: "outstation",
    headline: "Test without <em>any hardware</em>",
    sub: "Run a built-in outstation from a feeder template with live simulated values and an event buffer.",
    init,
    async setup(page) {
      await page.evaluate(() => {
        const s = [...document.querySelectorAll("select")].find((el) => [...el.options].some((o) => o.textContent === "Outstation"));
        s.setAttribute("data-mode", "1");
      }).catch(() => {});
      const clicked = await page.evaluate(() => {
        const b = [...document.querySelectorAll("button, [role=tab], [role=radio]")].find((el) => el.textContent.trim().startsWith("Outstation"));
        if (b) { b.click(); return true; }
        return false;
      });
      if (!clicked) throw new Error("mode switch not found");
      await sleep(500);
      await clickButton(page, "Start server");
      await sleep(600);
      await page.evaluate(() => {
        const s = [...document.querySelectorAll("select")].find((el) => [...el.options].some((o) => o.value === "feeder"));
        s.setAttribute("data-template", "1");
      });
      await page.select("[data-template]", "feeder");
      await sleep(600);
      await clickButton(page, "Simulate");
      for (let i = 0; i < 6; i++) {
        await page.evaluate((i) => window.__mock.emit("outstation-log", { timestamp: Date.now(), function_code: 1, message: ["READ g30v4 index 0-9 (class 0 poll)", "READ g1v2 index 0-1", "READ g20v1 index 0-3", "Response: 10 analog, 2 binary, 4 counter", "READ class 1 events", "Response: 4 events, confirm received"][i] }), i);
        await sleep(100);
      }
      await growTable(page, 40);
      await sleep(3000);
    },
  },
  {
    id: "trends",
    headline: "Trend any point <em>over time</em>",
    sub: "Add points to the trend chart and watch currents and voltages move together.",
    init,
    async setup(page) {
      await connect(page);
      await growTable(page, 40);
      for (const n of ["Current_A", "Current_B", "Current_C"]) {
        await page.click(`button[aria-label="Add ${n} to trend"]`);
        await sleep(700);
      }
      await page.evaluate(() => {
        const t = [...document.querySelectorAll('[role="tab"], button')].find((el) => el.textContent.trim().startsWith("Trend"));
        t.click();
      });
      await sleep(22000);
    },
  },
];
