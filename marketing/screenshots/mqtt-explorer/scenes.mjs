// App Store scenes for MQTT Explorer. Mock data models a bottling plant publishing to a broker
// (client_connect / client_subscribe / client_publish emit "client-message" like the Rust side),
// plus a local broker with a few devices publishing to it.

import { pickOption } from "../pick.mjs";

export const uiScale = 1;

// [topic, factory(i) -> payload string, retain]
const J = (o) => JSON.stringify(o);
const CATALOG = [
  ["factory/line1/filler/speed", (i) => String(598 + ((i * 7) % 9)), false],
  ["factory/line1/filler/level", (i) => String(+(71.4 - i * 0.6).toFixed(1)), false],
  ["factory/line1/filler/state", (i) => J({ state: "running", speed: 598 + ((i * 7) % 9) }), false],
  ["factory/line1/capper/torque", (i) => J({ value: +(2.31 + ((i * 3) % 5) * 0.02).toFixed(2), unit: "Nm" }), false],
  ["factory/line1/labeler/status", () => "RUNNING", true],
  ["factory/line1/conveyor/speed", (i) => String(+(1.82 + ((i * 5) % 7) * 0.01).toFixed(2)), false],
  ["factory/line1/count/good", (i) => String(18420 + i * 38), false],
  ["factory/line1/count/reject", (i) => String(212 + Math.floor(i / 2)), false],
  ["factory/line2/filler/speed", (i) => String(540 + ((i * 11) % 13)), false],
  ["factory/line2/filler/level", (i) => String(+(64.8 - i * 0.5).toFixed(1)), false],
  ["factory/line2/capper/torque", (i) => J({ value: +(2.28 + ((i * 7) % 5) * 0.02).toFixed(2), unit: "Nm" }), false],
  ["factory/line2/labeler/status", () => "IDLE", true],
  ["factory/line2/count/good", (i) => String(15377 + i * 33), false],
  ["factory/utilities/air/pressure", (i) => String(+(6.42 + ((i * 3) % 4) * 0.03).toFixed(2)), false],
  ["factory/utilities/chiller/supply_temp", (i) => String(+(7.1 + ((i * 5) % 6) * 0.05).toFixed(2)), false],
  ["factory/utilities/power/kw", (i) => String(+(184.2 + ((i * 13) % 17) * 0.8).toFixed(1)), false],
  ["factory/alarms/line2/low_level", () => J({ active: false }), true],
  ["factory/env/warehouse/temp", (i) => String(+(21.3 + ((i * 3) % 5) * 0.1).toFixed(1)), false],
  ["factory/env/warehouse/humidity", (i) => String(+(48.2 + ((i * 7) % 5) * 0.3).toFixed(1)), false],
];

const BROKER_TOPICS = [
  ["sensors/greenhouse/temp", "24.6", false],
  ["sensors/greenhouse/humidity", "63.1", false],
  ["sensors/greenhouse/co2", "812", false],
  ["devices/gateway-01/status", "online", true],
  ["devices/gateway-02/status", "online", true],
  ["devices/gateway-01/telemetry", '{"rssi":-61,"uptime":86412,"fw":"2.4.1"}', false],
  ["home/livingroom/light/state", "ON", true],
  ["home/livingroom/light/brightness", "72", false],
  ["home/garage/door", "closed", true],
  ["energy/meter1/power", "1.482", false],
];

function mockMain(D) {
  const T0 = Date.UTC(2026, 9, 21, 10, 14, 0);
  const realNow = Date.now.bind(Date);
  const r0 = realNow();
  window.__skew = 0;
  Date.now = () => T0 + (realNow() - r0) + window.__skew;
  Object.defineProperty(window.screen, "width", { get: () => 2560 });
  Object.defineProperty(window.screen, "height", { get: () => 1600 });

  const cbs = new Map();
  const listeners = new Map();
  let nextCb = 1;
  let nextEv = 1;
  window.__emit = (event, payload) => {
    // React StrictMode (dev server) leaks one extra listener per effect; only the newest one is live.
    const ls = listeners.get(event) ?? [];
    const l = ls[ls.length - 1];
    if (l) {
      const fn = cbs.get(l.handler);
      if (fn) fn({ event, id: l.id, payload });
    }
  };

  const match = (sub, topic) => {
    const s = sub.split("/");
    const t = topic.split("/");
    for (let i = 0; i < s.length; i++) {
      if (s[i] === "#") return true;
      if (i >= t.length) return false;
      if (s[i] !== "+" && s[i] !== t[i]) return false;
    }
    return s.length === t.length;
  };

  const subs = [];
  // Emit `rounds` rounds of the catalog (filled in by the scene via window.__factories).
  window.__feed = (rounds) => {
    for (let r = 0; r < rounds; r++) {
      window.__round = (window.__round ?? 0) + 1;
      window.__skew += 1700;
      for (const [topic, payload, retain] of window.__factories(window.__round)) {
        if (subs.some((s) => match(s, topic))) window.__emit("client-message", { topic, payload, qos: 0, retain });
      }
    }
  };

  const broker = { running: false, topics: new Map() };
  const brokerList = () => [...broker.topics.entries()].map(([topic, v]) => ({ topic, last_payload: v.payload, last_update: v.ts, message_count: v.count, retain: v.retain }));

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
    platform: () => "macos",
    client_connect: () => null,
    client_disconnect: () => null,
    client_subscribe: ({ topic }) => {
      subs.push(topic);
      return 0;
    },
    client_unsubscribe: ({ topic }) => {
      const i = subs.indexOf(topic);
      if (i >= 0) subs.splice(i, 1);
      return null;
    },
    client_publish: ({ message }) => {
      window.__skew += 400;
      if (subs.some((s) => match(s, message.topic))) {
        window.__emit("client-message", { topic: message.topic, payload: message.payload, qos: 0, retain: !!message.retained });
      }
      return null;
    },
    broker_start: () => {
      broker.running = true;
      for (const [t, p, r] of D.brokerTopics) broker.topics.set(t, { payload: p, ts: Date.now() - 4000, count: 1, retain: r });
      return null;
    },
    broker_stop: () => {
      broker.running = false;
      return null;
    },
    broker_get_status: () => (broker.running ? "running" : "stopped"),
    broker_get_topics: () => brokerList(),
    broker_publish: ({ topic, payload }) => {
      const ex = broker.topics.get(topic);
      broker.topics.set(topic, { payload, ts: Date.now(), count: (ex?.count ?? 0) + 1, retain: ex?.retain ?? false });
      window.__emit("broker-message", { topic, payload, timestamp: Date.now(), retain: false });
      return null;
    },
    open_url: () => null,
  };

  window.__brokerFeed = (rounds) => {
    for (let r = 0; r < rounds; r++) {
      window.__skew += 900;
      const ts = Date.now();
      for (const [t, p] of window.__brokerRound((window.__bround = (window.__bround ?? 0) + 1))) {
        const ex = broker.topics.get(t);
        broker.topics.set(t, { payload: p, ts, count: (ex?.count ?? 0) + 1, retain: ex?.retain ?? false });
        window.__emit("broker-message", { topic: t, payload: p, timestamp: ts, retain: false });
      }
    }
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

// Payload generators are functions, so they are injected as source next to the serialised data.
const mockData = { brokerTopics: BROKER_TOPICS };

export const mock = `(${mockMain.toString()})(${JSON.stringify(mockData)});
(() => {
  const J = ${J.toString()};
  const CATALOG = ${JSON.stringify(CATALOG.map(([t]) => t))};
  const FN = [${CATALOG.map(([, f]) => f.toString()).join(",\n")}];
  const RET = ${JSON.stringify(CATALOG.map(([, , r]) => r))};
  window.__factories = (i) => CATALOG.map((t, k) => [t, FN[k](i), RET[k]]);
  const BR = [${[
    `(i) => String(+(24.6 + ((i * 3) % 5) * 0.1).toFixed(1))`,
    `(i) => String(+(63.1 - ((i * 7) % 5) * 0.2).toFixed(1))`,
    `(i) => String(812 + ((i * 11) % 9) * 3)`,
    `(i) => J({ rssi: -61 - (i % 4), uptime: 86412 + i * 5, fw: "2.4.1" })`,
    `(i) => String(72 + ((i * 3) % 4))`,
    `(i) => String(+(1.482 + ((i * 7) % 6) * 0.011).toFixed(3))`,
  ].join(",")}];
  const BT = ["sensors/greenhouse/temp","sensors/greenhouse/humidity","sensors/greenhouse/co2","devices/gateway-01/telemetry","home/livingroom/light/brightness","energy/meter1/power"];
  window.__brokerRound = (i) => BT.map((t, k) => [t, BR[k](i)]);
})();`;

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

async function prep(page) {
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.addStyleTag({ content: "#root > div { height: calc(100vh - 50px) !important; }" });
  await layout(page);
}

// Narrower side panels and a taller table, so payload columns have room.
async function layout(page) {
  const drag = async (cursor, dx, dy) => {
    const r = await page.evaluate((cursor) => {
      const el = [...document.querySelectorAll("div")].find((d) => d.style.cursor === cursor && d.style.flexShrink === "0");
      if (!el) return null;
      const b = el.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    }, cursor);
    if (!r) return;
    await page.mouse.move(r.x, r.y);
    await page.mouse.down();
    await page.mouse.move(r.x + dx, r.y + dy, { steps: 6 });
    await page.mouse.up();
    await page.mouse.move(5, 5);
  };
  await drag("col-resize", -50, 0);
  await drag("row-resize", 0, 150);
}

async function typeInto(page, selector, value, index = 0) {
  const inputs = await page.$$(selector);
  await inputs[index].click();
  await page.keyboard.down("Control");
  await page.keyboard.press("a");
  await page.keyboard.up("Control");
  await page.keyboard.type(String(value));
}

async function connectAndSubscribe(page, filter = "factory/#", rounds = 4) {
  await typeInto(page, "aside input", "broker.acme-bottling.net", 1);
  await typeInto(page, "aside input", "1883", 2);
  await page.evaluate(() => document.activeElement?.blur());
  await clickText(page, "aside button", "Connect", "equals");
  await page.waitForFunction(() => [...document.querySelectorAll("aside button")].some((b) => b.textContent.trim() === "Subscribe"), { timeout: 5000 });
  await typeInto(page, 'aside input[aria-label="Topic filter"]', filter);
  await clickText(page, "aside button", "Subscribe", "equals");
  await sleep(200);
  await page.evaluate((n) => window.__feed(n), rounds);
  await sleep(900);
}

export const scenes = [
  {
    id: "topics",
    headline: "See every topic <em>as it updates</em>",
    sub: "Subscribe with wildcards and watch the latest value of each topic change live.",
    setup: async (page) => {
      await prep(page);
      await connectAndSubscribe(page);
    },
  },
  {
    id: "tree",
    headline: "Explore your broker as a <em>topic tree</em>",
    sub: "Plants, lines and devices fold into a hierarchy with the latest payload on every leaf.",
    setup: async (page) => {
      await prep(page);
      await connectAndSubscribe(page);
      await clickText(page, '[role="tab"]', "Tree");
      await sleep(1800);
    },
  },
  {
    id: "json",
    headline: "Read payloads <em>your way</em>",
    sub: "Switch between raw text, formatted JSON, Hex and Base64 for every message.",
    setup: async (page) => {
      await prep(page);
      await connectAndSubscribe(page, "factory/line1/#");
      await pickOption(page, "Raw", "JSON");
      await sleep(300);
    },
  },
  {
    id: "history",
    headline: "Follow the <em>message stream</em>",
    sub: "Every message with its topic, QoS, retain flag and timestamp, newest at the bottom.",
    setup: async (page) => {
      await prep(page);
      await connectAndSubscribe(page, "factory/line2/#", 5);
      await clickText(page, '[role="tab"]', "History");
      await sleep(300);
    },
  },
  {
    id: "broker",
    headline: "Test against your own <em>local broker</em>",
    sub: "Start a broker on this Mac, point your devices at it and watch the topics arrive.",
    setup: async (page) => {
      await prep(page);
      await clickText(page, "aside button", "Broker");
      await sleep(200);
      await clickText(page, "aside button", "Start broker", "equals");
      await sleep(300);
      await page.evaluate(() => window.__brokerFeed(6));
      await sleep(1500);
    },
  },
];

/** Extra states for render-selects.mjs: selects the App Store scenes never show enabled. */
export const selectScenes = [
  {
    id: "advanced",
    async setup(page) {
      await prep(page);
      await clickText(page, "aside button", "TLS, LWT, Keep-alive");
      await sleep(300);
      await clickText(page, "aside button", "Last Will and Testament");
      await sleep(300);
    },
  },
  {
    id: "decoders",
    // The mock has no protobuf engine: a saved schema with one topic rule, and proto_inspect / proto_configure answered here.
    init: `localStorage.setItem("mqtt-explorer:proto:v1", ${JSON.stringify(JSON.stringify({ source: 'syntax = "proto3";\nmessage Reading { string sensor = 1; double value = 2; }\nmessage Status { bool ok = 1; }', rules: [{ filter: "factory/line1/#", message: "Reading" }] }))});
      const invoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = async (cmd, args) => (cmd === "proto_inspect" ? ["Reading", "Status"] : cmd === "proto_configure" ? null : invoke(cmd, args));`,
    async setup(page) {
      await prep(page);
      await connectAndSubscribe(page);
      await clickText(page, "button", "Decoders");
      await page.waitForSelector('[aria-label="Message type"]');
      await sleep(300);
    },
  },
];
