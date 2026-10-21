// App Store scenes for OPC UA Explorer. Mock data models a small process plant (boiler, pump, tank, packaging
// line) served by one OPC UA server at opc.tcp://192.168.20.30:4840, plus the built-in server simulator.
//
// Mock command shapes follow apps/opcua-explorer/src-tauri/src/main.rs and crates/mt-opcua/src/types.rs:
//   browse_nodes -> { node_id, name, data_type }[], read_tags -> { node_id, name, value, timestamp }[],
//   browse_children / browse_references / read_attributes (AttributeRead rows in the order the Rust client reads them),
//   create_subscription -> id, create_monitored_items -> count, "opcua://notification" events
//   ({ node_id, value, source_ts, server_ts, status } with Unix-ms timestamps), server_read_nodes for the simulator.

export const uiScale = 1.1;

function mockMain() {
  const T0 = new Date(2026, 9, 21, 10, 14, 0).getTime();
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
  const emit = (event, payload) => {
    for (const l of listeners.get(event) ?? []) {
      const fn = cbs.get(l.handler);
      if (fn) fn({ event, id: l.id, payload });
    }
  };
  window.__emit = emit;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const smooth = (x) => {
    const c = Math.min(1, Math.max(0, x));
    return c * c * (3 - 2 * c);
  };

  // ── Plant model: [nodeId, browse name, display name, data type, description, EU, range, writable, value(t)] ──
  const step = (t, period, lo, hi, phase = 0) => {
    // Slow ramp between two levels with eased steps, like a valve being repositioned.
    const k = Math.floor(t / period + phase);
    const f = t / period + phase - k;
    const a = k % 2 === 0 ? lo : hi;
    const b = k % 2 === 0 ? hi : lo;
    return a + (b - a) * smooth((f - 0.35) / 0.4);
  };
  const SIG = [
    ["ns=2;s=Boiler.Temperature", "BoilerTemp", "Temperature", "Double", "Boiler outlet temperature", "°C", [0, 250], false, (t) => 184.6 + 1.1 * Math.sin(t / 7) + 0.4 * Math.sin(t / 2.3), 1, "Boiler"],
    ["ns=2;s=Boiler.SteamPressure", "SteamPressure", "SteamPressure", "Double", "Steam header pressure", "bar", [0, 16], false, (t) => 10.42 + 0.08 * Math.sin(t / 9) + 0.025 * Math.sin(t / 2.9), 2, "Boiler"],
    ["ns=2;s=Boiler.DrumLevel", "DrumLevel", "DrumLevel", "Double", "Steam drum water level", "%", [0, 100], false, (t) => 56.2 + 2.2 * Math.sin(t / 11) + 0.5 * Math.sin(t / 3.1), 1, "Boiler"],
    ["ns=2;s=Boiler.BurnerOn", "BurnerOn", "BurnerOn", "Boolean", "Burner firing status", "", null, false, () => true, 0, "Boiler"],
    ["ns=2;s=Pump1.Speed", "Pump1Speed", "Speed", "Double", "Feed pump speed", "rpm", [0, 3000], true, (t) => 1480 + 14 * Math.sin(t / 8) + 4 * Math.sin(t / 1.9), 0, "Pump1"],
    ["ns=2;s=Pump1.MotorCurrent", "Pump1Current", "MotorCurrent", "Double", "Pump motor current", "A", [0, 40], false, (t) => 18.4 + 0.7 * Math.sin(t / 8 + 0.6) + 0.15 * Math.sin(t / 2.1), 2, "Pump1"],
    ["ns=2;s=Pump1.Running", "Pump1Running", "Running", "Boolean", "Pump running status", "", null, false, () => true, 0, "Pump1"],
    ["ns=2;s=Pump1.RunHours", "Pump1Hours", "RunHours", "UInt32", "Accumulated run hours", "h", null, false, (t) => 12873 + Math.floor(t / 3600), 0, "Pump1"],
    ["ns=2;s=Tank1.Level", "TankLevel", "Level", "Double", "Storage tank level", "%", [0, 100], false, (t) => 72.4 + 4.2 * Math.sin(t / 14) + 1.1 * Math.sin(t / 3.7), 1, "Tank1"],
    ["ns=2;s=Tank1.Temperature", "TankTemp", "Temperature", "Double", "Storage tank temperature", "°C", [0, 120], false, (t) => 41.2 + 0.35 * Math.sin(t / 16) + 0.08 * Math.sin(t / 2.7), 1, "Tank1"],
    ["ns=2;s=Tank1.InletValve", "InletValve", "InletValve", "Double", "Inlet valve position", "%", [0, 100], true, (t) => step(t, 30, 52, 78, 0.2) + 0.25 * Math.sin(t / 2.2), 1, "Tank1"],
    ["ns=2;s=Tank1.OutletValve", "OutletValve", "OutletValve", "Double", "Outlet valve position", "%", [0, 100], true, (t) => step(t, 24, 34, 47, 0.7) + 0.2 * Math.sin(t / 1.7), 1, "Tank1"],
    ["ns=2;s=Line1.PartCount", "PartCount", "PartCount", "UInt32", "Parts produced this shift", "", null, false, (t) => 48210 + Math.floor(t * 2.1), 0, "Line1"],
    ["ns=2;s=Line1.RejectCount", "RejectCount", "RejectCount", "UInt32", "Parts rejected this shift", "", null, false, (t) => 312 + Math.floor(t / 41), 0, "Line1"],
    ["ns=2;s=Line1.Speed", "LineSpeed", "Speed", "Double", "Line speed", "parts/min", [0, 200], true, (t) => 126 + 3.2 * Math.sin(t / 10) + 1.1 * Math.sin(t / 2.4), 1, "Line1"],
    ["ns=2;s=Line1.Fault", "LineFault", "Fault", "Boolean", "Line fault active", "", null, false, () => false, 0, "Line1"],
  ].map(([id, name, display, dt, desc, eu, range, writable, fn, dec, parent]) => ({ id, name, display, dt, desc, eu, range, writable, fn, dec, parent }));
  const byId = new Map(SIG.map((s) => [s.id, s]));
  const virtualT = () => (Date.now() - T0) / 1000;
  const valueOf = (s, t = virtualT()) => {
    const v = s.fn(t);
    return typeof v === "number" && s.dt === "Double" ? +v.toFixed(s.dec || 1) : v;
  };

  // ── Address space ──
  const OBJ = (id, name) => ({ node_id: id, browse_name: name, display_name: name, node_class: "Object", data_type: "", is_variable: false });
  const VAR = (s) => ({ node_id: s.id, browse_name: s.name, display_name: s.display, node_class: "Variable", data_type: s.dt, is_variable: true });
  const PARENTS = ["Boiler", "Pump1", "Tank1", "Line1"];
  const children = {
    "": [OBJ("i=2253", "Server"), OBJ("ns=2;s=Plant", "Plant")],
    "i=2253": [
      { node_id: "i=2256", browse_name: "ServerStatus", display_name: "ServerStatus", node_class: "Variable", data_type: "ServerStatusDataType", is_variable: true },
      { node_id: "i=2255", browse_name: "NamespaceArray", display_name: "NamespaceArray", node_class: "Variable", data_type: "String", is_variable: true },
      OBJ("i=2268", "ServerCapabilities"),
    ],
    "ns=2;s=Plant": PARENTS.map((p) => OBJ("ns=2;s=" + p, p)),
  };
  for (const p of PARENTS) children["ns=2;s=" + p] = SIG.filter((s) => s.parent === p).map(VAR);

  const accessLevel = (s) => (s.writable ? 3 : 1);
  const readAttributes = ({ nodeId }) => {
    const s = byId.get(nodeId);
    if (!s) {
      const o = Object.values(children).flat().find((n) => n.node_id === nodeId);
      const nm = o?.browse_name ?? nodeId;
      return [
        { attribute: "BrowseName", value: (nodeId.startsWith("ns=2") ? "2:" : "0:") + nm, status: "Good" },
        { attribute: "DisplayName", value: nm, status: "Good" },
        { attribute: "Description", value: nm === "Plant" ? "Plant floor equipment" : "", status: "Good" },
        { attribute: "NodeClass", value: o?.node_class ?? "Object", status: "Good" },
      ];
    }
    const rows = [
      { attribute: "Value", value: valueOf(s), status: "Good" },
      { attribute: "AccessLevel", value: accessLevel(s), status: "Good" },
      { attribute: "UserAccessLevel", value: accessLevel(s), status: "Good" },
      { attribute: "BrowseName", value: "2:" + s.name, status: "Good" },
      { attribute: "DisplayName", value: s.display, status: "Good" },
      { attribute: "Description", value: s.desc, status: "Good" },
      { attribute: "NodeClass", value: "Variable", status: "Good" },
      { attribute: "DataType", value: s.dt, status: "Good" },
    ];
    if (s.range) rows.push({ attribute: "EURange", value: { low: s.range[0], high: s.range[1] }, status: "Good" });
    if (s.eu) rows.push({ attribute: "EngineeringUnits", value: { unit_id: 4408652, display_name: s.eu, namespace_uri: "http://www.opcfoundation.org/UA/units/un/cefact", description: s.eu }, status: "Good" });
    return rows;
  };
  const ref = (type, fwd, id, name, cls) => ({ reference_type: type, is_forward: fwd, node_id: id, browse_name: name, display_name: name, node_class: cls });
  const browseReferences = ({ nodeId }) => {
    const s = byId.get(nodeId);
    if (!s) {
      return (children[nodeId] ?? []).map((c) => ref("Organizes", true, c.node_id, c.display_name, c.node_class));
    }
    return [
      ref("HasComponent", false, "ns=2;s=" + s.parent, s.parent, "Object"),
      ref("HasTypeDefinition", true, "i=2368", "AnalogItemType", "VariableType"),
      ref("HasProperty", true, nodeId + ".EURange", "EURange", "Variable"),
      ref("HasProperty", true, nodeId + ".EngineeringUnits", "EngineeringUnits", "Variable"),
    ];
  };

  // ── Subscriptions: one ticker pushes data-change notifications for every monitored node ──
  const subs = new Map();
  let nextSub = 1;
  const lastSent = new Map();
  const notify = (nodeId, force) => {
    const s = byId.get(nodeId);
    if (!s) return;
    const v = valueOf(s);
    if (!force && lastSent.get(nodeId) === v) return;
    lastSent.set(nodeId, v);
    const ts = Date.now();
    emit("opcua://notification", { node_id: nodeId, value: v, source_ts: ts - 3, server_ts: ts, status: "Good" });
  };
  const tick = () => {
    const ids = new Set();
    for (const nodes of subs.values()) nodes.forEach((n) => ids.add(n));
    ids.forEach((id) => notify(id, false));
  };
  setInterval(tick, 1000);
  window.__advance = async (n) => {
    for (let i = 0; i < n; i++) {
      window.__skew += 1000;
      tick();
      await sleep(25);
    }
    await sleep(120);
  };

  // ── Server simulator ──
  const server = { running: false, profile: null, overrides: {} };
  const simValue = (def, i, t) => {
    if (def.isBool) return def.base >= 0.5;
    let v = def.base + def.variance * Math.sin((t * 1000 / def.period) * 2 * Math.PI + i) + def.noise * Math.sin(t * 2.3 + i * 5.1);
    if (def.min !== undefined && def.min !== null) v = Math.max(def.min, v);
    if (def.max !== undefined && def.max !== null) v = Math.min(def.max, v);
    return +v.toFixed(2);
  };
  const serverNodes = () => {
    const t = virtualT();
    if (!server.profile) return [];
    return server.profile.registers.map((r, i) => ({
      node_id: r.nodeId, name: r.name, data_type: r.dataType,
      value: server.overrides[r.nodeId] ?? simValue(r, i, t), timestamp: Date.now(),
    }));
  };

  const ENDPOINT = "opc.tcp://192.168.20.30:4840";
  const POL = "http://opcfoundation.org/UA/SecurityPolicy#";
  const handlers = {
    store_status: () => ({ store: false, pro: true, reason: "build", price: null }),
    platform: () => "macos",
    open_url: () => null,
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
    connect: async () => {
      await sleep(350);
      return null;
    },
    disconnect: () => {
      subs.clear();
      return null;
    },
    browse_nodes: () => SIG.map((s) => ({ node_id: s.id, name: s.name, data_type: s.dt })),
    read_tags: ({ tags }) => tags.map((t) => {
      const s = byId.get(t.node_id);
      return { node_id: t.node_id, name: s?.name ?? t.name, value: s ? valueOf(s) : null, timestamp: Date.now() };
    }),
    write_tag: () => null,
    browse_children: ({ nodeId }) => children[nodeId] ?? [],
    browse_references: browseReferences,
    read_attributes: readAttributes,
    read_namespaces: () => [
      { index: 0, uri: "http://opcfoundation.org/UA/" },
      { index: 1, uri: "urn:plant-gateway:UA:Server" },
      { index: 2, uri: "urn:plant-gateway:Plant" },
      { index: 3, uri: "http://opcfoundation.org/UA/DI/" },
    ],
    get_endpoints: () => [
      { url: ENDPOINT, security_policy_uri: POL + "None", security_mode: "MessageSecurityMode::None", transport_profile_uri: "http://opcfoundation.org/UA-Profile/Transport/uatcp-uasc-uabinary", security_level: 0, user_token_policies: ["Anonymous", "UserName"] },
      { url: ENDPOINT, security_policy_uri: POL + "Basic256Sha256", security_mode: "MessageSecurityMode::Sign", transport_profile_uri: "http://opcfoundation.org/UA-Profile/Transport/uatcp-uasc-uabinary", security_level: 3, user_token_policies: ["UserName", "Certificate"] },
      { url: ENDPOINT, security_policy_uri: POL + "Basic256Sha256", security_mode: "MessageSecurityMode::SignAndEncrypt", transport_profile_uri: "http://opcfoundation.org/UA-Profile/Transport/uatcp-uasc-uabinary", security_level: 4, user_token_policies: ["UserName", "Certificate"] },
    ],
    find_servers: () => [],
    cert_list: () => [],
    create_subscription: () => nextSub++,
    create_monitored_items: ({ subscriptionId, items }) => {
      subs.set(subscriptionId, items.map((i) => i.node_id));
      setTimeout(() => items.forEach((i) => notify(i.node_id, true)), 60);
      return items.length;
    },
    delete_subscription: ({ subscriptionId }) => {
      subs.delete(subscriptionId);
      return null;
    },
    server_start: async () => {
      await sleep(150);
      server.running = true;
      return "opc.tcp://0.0.0.0:4840/";
    },
    server_stop: () => {
      server.running = false;
      server.profile = null;
      return null;
    },
    server_read_nodes: () => serverNodes(),
    server_write_node: ({ nodeId, value }) => {
      server.overrides[nodeId] = value;
      return null;
    },
    sim_start: ({ profile }) => {
      server.profile = profile;
      server.overrides = {};
      return null;
    },
    sim_stop: () => {
      server.profile = null;
      return null;
    },
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

export const mock = `(${mockMain.toString()})();`;

// ───────────────────────────── scene helpers ─────────────────────────────

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function clickText(page, selector, text, what = "startsWith") {
  const ok = await page.evaluate(
    (selector, text, what) => {
      const el = [...document.querySelectorAll(selector)].find((e) => {
        const t = e.textContent.trim();
        return what === "equals" ? t === text : t.startsWith(text);
      });
      if (!el || el.disabled) return false;
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
  await page.addStyleTag({
    content: `#root > div { height: calc(100vh - 50px) !important; }
      *::-webkit-scrollbar { display: none; } * { scrollbar-width: none; }`,
  });
}

async function typeInto(handle, value) {
  await handle.click();
  await handle.evaluate((el) => el.select());
  await handle.type(String(value));
}

async function connect(page) {
  await clickText(page, "button", "Client");
  await page.waitForSelector("aside select", { timeout: 5000 });
  await sleep(200);
  const inputs = await page.$$("aside input");
  await typeInto(inputs[0], "opc.tcp://192.168.20.30:4840");
  const selects = await page.$$("aside select");
  await selects[0].select("Basic256Sha256");
  await selects[1].select("SignAndEncrypt");
  await typeInto(inputs[1], "operator");
  await typeInto(inputs[2], "plantfloor");
  await clickText(page, "aside button", "Connect", "equals");
  await page.waitForFunction(() => [...document.querySelectorAll("aside button")].some((b) => b.textContent.trim().startsWith("Subscriptions")), { timeout: 5000 });
  await sleep(1500);
}

async function dragHandle(page, cursor, dx, dy) {
  const box = await page.evaluate((cursor) => {
    const h = [...document.querySelectorAll("div")].find((d) => d.style.cursor === cursor);
    const r = h.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  }, cursor);
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  await page.mouse.move(box.x + dx, box.y + dy, { steps: 8 });
  await page.mouse.up();
  await sleep(250);
}

async function trendButtons(page, names) {
  for (const name of names) {
    // A notification landing in the same render can drop a just-added series, so confirm and retry.
    for (let attempt = 0; attempt < 5; attempt++) {
      const state = await page.evaluate((name) => {
        const add = document.querySelector(`button[aria-label="Add ${name} to trend"]`);
        if (add) add.click();
        return add ? "clicked" : document.querySelector(`button[aria-label="Remove ${name} from trend"]`) ? "on" : "missing";
      }, name);
      if (state === "missing") throw new Error("no trend button for " + name);
      await sleep(400);
      if (state === "on") break;
    }
    const on = await page.evaluate((name) => !!document.querySelector(`button[aria-label="Remove ${name} from trend"]`), name);
    if (!on) throw new Error("trend series not kept for " + name);
  }
}

// ───────────────────────────── scenes ─────────────────────────────

export const scenes = [
  {
    id: "live",
    headline: "Watch your plant data <em>live</em>",
    sub: "Connect to any OPC UA server over a secure channel and see every tag update the moment it changes.",
    settleMs: 900,
    setup: async (page) => {
      await connect(page);
      await dragHandle(page, "row-resize", 0, 215);
      await page.evaluate(() => window.__advance(6));
    },
  },
  {
    id: "browse",
    headline: "Browse the whole <em>address space</em>",
    sub: "Walk the node tree, then inspect each node's attributes, access level, units and references.",
    settleMs: 700,
    setup: async (page) => {
      await connect(page);
      await clickText(page, "aside button", "Browse", "equals");
      await page.waitForSelector('[role="button"][aria-label^="Expand"]', { timeout: 5000 });
      await sleep(250);
      for (const name of ["Plant", "Pump1"]) {
        await page.evaluate((name) => document.querySelector(`[aria-label="Expand ${name}"]`).click(), name);
        await sleep(300);
      }
      // Select the pump speed variable.
      await page.evaluate(() => {
        const row = [...document.querySelectorAll(".vt-dialog span")].find((e) => e.textContent === "Speed" && e.nextElementSibling?.textContent.includes("Double"));
        row.click();
      });
      await sleep(400);
    },
  },
  {
    id: "trend",
    headline: "Trend any value as it <em>changes</em>",
    sub: "Add tags to a live chart with one click and compare several signals over 30 seconds to 5 minutes.",
    setup: async (page) => {
      await connect(page);
      await dragHandle(page, "row-resize", 0, -60);
      await trendButtons(page, ["TankLevel", "DrumLevel", "InletValve", "OutletValve"]);
      await clickText(page, "button", "Trend");
      await page.evaluate(() => window.__advance(75));
    },
  },
  {
    id: "subscriptions",
    headline: "Tune every <em>subscription</em>",
    sub: "Set sampling and publishing intervals, queue size and deadband, then watch notifications arrive.",
    settleMs: 800,
    setup: async (page) => {
      await connect(page);
      await clickText(page, "aside button", "Subscriptions");
      await page.waitForSelector(".vt-dialog", { timeout: 3000 });
      const nodes = ["ns=2;s=Boiler.Temperature", "ns=2;s=Boiler.SteamPressure", "ns=2;s=Pump1.MotorCurrent", "ns=2;s=Tank1.Level", "ns=2;s=Line1.Speed"];
      for (const n of nodes) {
        const input = await page.$('.vt-dialog input[aria-label="Node ID to monitor"]');
        await input.click();
        await input.type(n);
        await page.keyboard.press("Enter");
        await sleep(80);
      }
      const num = await page.$$('.vt-dialog input[type="number"]');
      await typeInto(num[2], 20);
      await page.evaluate(() => document.querySelectorAll(".vt-dialog select")[2].dispatchEvent(new Event("change", { bubbles: true })));
      const sels = await page.$$(".vt-dialog select");
      await sels[2].select("1");
      await sleep(100);
      await typeInto((await page.$$('.vt-dialog input[type="number"]'))[3], 0.2);
      await clickText(page, ".vt-dialog button", "Start", "equals");
      await sleep(300);
      await page.evaluate(() => window.__advance(40));
    },
  },
  {
    id: "simulator",
    headline: "Test without <em>any hardware</em>",
    sub: "Start the built-in OPC UA server with a reactor template and connect to it from any client.",
    settleMs: 900,
    setup: async (page) => {
      await clickText(page, "aside button", "Start server", "equals");
      await sleep(400);
      await page.select("aside select", "reactor");
      await sleep(200);
      await clickText(page, "aside button", "Start simulation", "equals");
      await sleep(500);
      await page.evaluate(() => window.__advance(30));
      await dragHandle(page, "row-resize", 0, -30);
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
