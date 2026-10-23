// App Store scenes for NATS Explorer. Mock data models an order-processing platform on a NATS server:
// core subjects (orders.*, payments.*, inventory.*), JetStream streams, a KV config bucket and an object store.
//
// Mock command shapes follow apps/nats-explorer/src-tauri/src/lib.rs and src/nats-types.ts.

export const uiScale = 1;

const T0 = Date.UTC(2026, 9, 21, 10, 14, 0);

const STREAMS = [
  { name: "ORDERS", subjects: ["orders.>"], messages: 184220, bytes: 96468992, max_messages: -1, consumers: 3, storage: "file" },
  { name: "PAYMENTS", subjects: ["payments.*"], messages: 91874, bytes: 41943040, max_messages: 500000, consumers: 2, storage: "file" },
  { name: "INVENTORY", subjects: ["inventory.>"], messages: 12408, bytes: 5767168, max_messages: 100000, consumers: 1, storage: "memory" },
  { name: "SHIPMENTS", subjects: ["shipments.>"], messages: 48310, bytes: 22020096, max_messages: -1, consumers: 2, storage: "file" },
  { name: "AUDIT", subjects: ["audit.>"], messages: 703115, bytes: 318767104, max_messages: 2000000, consumers: 1, storage: "file" },
].map((s) => ({ ...s, description: null, max_age_ns: 0, created: "2026-08-03T09:12:00Z" }));

const CUSTOMERS = ["Northwind", "Acme Retail", "Globex", "Initech", "Umbrella", "Hooli", "Stark Supply", "Wayne Parts"];
const SKUS = ["SKU-4410", "SKU-2087", "SKU-9921", "SKU-1305", "SKU-7742", "SKU-5518"];

const MSGS = [
  ["orders.created", { id: "ord_10482", customer: "Northwind", items: [{ sku: "SKU-4410", qty: 12 }, { sku: "SKU-2087", qty: 3 }], total: 1284.5, currency: "EUR" }],
  ["payments.authorized", { order: "ord_10481", method: "card", amount: 342.0, currency: "EUR", auth: "A-7731" }],
  ["inventory.reserved", { sku: "SKU-4410", warehouse: "AMS-2", qty: 12, remaining: 388 }],
  ["orders.created", { id: "ord_10483", customer: "Acme Retail", items: [{ sku: "SKU-9921", qty: 40 }], total: 5120.0, currency: "EUR" }],
  ["shipments.dispatched", { order: "ord_10479", carrier: "DHL", tracking: "JD014600003SE", parcels: 2 }],
  ["orders.updated", { id: "ord_10478", status: "packed", by: "pick-station-4" }],
  ["payments.captured", { order: "ord_10480", amount: 89.9, currency: "EUR" }],
  ["inventory.low", { sku: "SKU-1305", warehouse: "RTM-1", remaining: 14, threshold: 25 }],
  ["orders.created", { id: "ord_10484", customer: "Globex", items: [{ sku: "SKU-7742", qty: 6 }, { sku: "SKU-5518", qty: 1 }], total: 760.2, currency: "EUR" }],
  ["orders.cancelled", { id: "ord_10470", reason: "customer_request" }],
  ["payments.authorized", { order: "ord_10482", method: "sepa", amount: 1284.5, currency: "EUR", auth: "A-7732" }],
  ["shipments.delivered", { order: "ord_10462", signedBy: "R. Visser", at: "2026-10-21T09:58:12Z" }],
  ["orders.created", { id: "ord_10485", customer: "Initech", items: [{ sku: "SKU-2087", qty: 9 }], total: 431.1, currency: "EUR" }],
  ["inventory.reserved", { sku: "SKU-9921", warehouse: "AMS-2", qty: 40, remaining: 160 }],
].map(([subject, p], i) => ({
  subject,
  reply: null,
  headers: i % 3 === 0 ? [["Nats-Msg-Id", "m-" + (9100 + i)], ["trace-id", "4bf92f35" + i]] : [],
  payload: JSON.stringify(p),
  timestamp_ms: T0 - (MSGS_LEN() - i) * 1700,
}));
function MSGS_LEN() { return 14; }

const KV = {
  "app-config": [
    ["checkout.max_items", "250"], ["checkout.currency", "EUR"], ["feature.express_shipping", "true"],
    ["feature.gift_wrap", "false"], ["limits.rate_per_minute", "1200"], ["warehouse.default", "AMS-2"],
    ["notify.slack_channel", "#ops-orders"], ["retry.backoff_ms", "250"],
  ],
  "sessions": [["sess_a91f", "{\"user\":\"u_204\"}"]],
};

const OBJS = {
  "invoices": [
    ["invoice-2026-10-0412.pdf", 184320], ["invoice-2026-10-0413.pdf", 176128], ["invoice-2026-10-0414.pdf", 191488],
    ["labels-batch-1021.zip", 2411520], ["catalog-2026Q4.csv", 904192],
  ],
};

function mockMain(D) {
  const T = D.t0;
  window.__msgs = D.msgs;
  const realNow = Date.now.bind(Date);
  const r0 = realNow();
  Date.now = () => T + (realNow() - r0);
  Object.defineProperty(window.screen, "width", { get: () => 2560 });
  Object.defineProperty(window.screen, "height", { get: () => 1600 });

  const cbs = new Map();
  const listeners = new Map();
  let nextCb = 1;
  let nextEv = 1;
  window.__emit = (event, payload) => {
    const ls = listeners.get(event) ?? [];
    const l = ls[ls.length - 1];
    if (l) {
      const fn = cbs.get(l.handler);
      if (fn) fn({ event, id: l.id, payload });
    }
  };
  let sid = 1;
  let serverRecentSent = false;
  const sys = {
    "$SYS.REQ.SERVER.PING": { server: { name: "nats-ams-1", host: "10.20.4.11", id: "NDK7XQ2", ver: "2.10.22", jetstream: true, time: "2026-10-21T10:14:00Z" }, statsz: { start: "2026-10-14T06:02:11Z", mem: 41943040, cores: 8, cpu: 3.2, connections: 214, total_connections: 5312, active_accounts: 3, subscriptions: 1840, sent: { msgs: 91204411, bytes: 28334112211 }, received: { msgs: 90117240, bytes: 27998120043 }, slow_consumers: 0 } },
    "$SYS.REQ.SERVER.PING.VARZ": { server_name: "nats-ams-1", version: "2.10.22", port: 4222, max_payload: 1048576, max_connections: 65536, ping_interval: 120000000000, in_msgs: 90117240, out_msgs: 91204411, mem: 41943040, uptime: "6d8h12m" },
    "$SYS.REQ.SERVER.PING.CONNZ": { num_connections: 214, total: 214, offset: 0, limit: 1024 },
    "$SYS.REQ.SERVER.PING.JSZ": { streams: 5, consumers: 9, messages: 1039927, bytes: 484966400, api: { total: 48120, errors: 3 } },
  };
  const sysEvents = [
    ["$SYS.ACCOUNT.SHOP.CONNECT", { client: { id: 4412, name: "orders-api-7", host: "10.20.4.52" } }],
    ["$SYS.ACCOUNT.SHOP.SERVER.CONNS", { conns: 214, leafnodes: 0 }],
    ["$SYS.SERVER.NDK7XQ2.STATSZ", { conns: 214, subs: 1840, mem: 41943040 }],
  ];

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
    client_connect: () => null,
    client_disconnect: () => null,
    client_get_recent: () => [],
    client_subscribe: () => "sub-" + sid++,
    client_unsubscribe: () => null,
    client_publish: () => null,
    client_request: ({ message }) => ({ subject: "_INBOX.x", headers: [], payload: JSON.stringify({ status: "ok" }), timestamp_ms: Date.now() }),
    server_start: () => null,
    server_stop: () => null,
    server_get_status: () => "running",
    server_get_subjects: () => D.subjects,
    server_get_recent: () => {
      if (serverRecentSent) return [];
      serverRecentSent = true;
      return D.msgs.map((m) => ({ subject: m.subject, payload: m.payload, timestamp: m.timestamp_ms, headers: m.headers }));
    },
    server_publish: () => null,
    js_list_streams: () => D.streams.map((s) => s.name),
    js_stream_info: ({ name }) => D.streams.find((s) => s.name === name) ?? null,
    js_stream_messages: ({ name }) => D.msgs.filter((m) => m.subject.startsWith(name.toLowerCase())).map((m, i) => ({ sequence: 184100 + i, subject: m.subject, payload: m.payload, timestamp_ms: m.timestamp_ms })),
    js_create_stream: () => null,
    js_delete_stream: () => null,
    js_purge_stream: () => null,
    js_create_consumer: () => null,
    js_consumer_info: ({ stream, durable }) => ({ stream, name: durable, durable: true, deliver_kind: "all", ack_policy: "explicit", pending: 0, num_ack_pending: 0, num_redelivered: 0 }),
    js_consumer_next: () => [],
    js_delete_consumer: () => null,
    kv_list: () => Object.keys(D.kv),
    kv_keys: ({ bucket }) => D.kv[bucket].map((e) => e[0]),
    kv_get: ({ bucket, key }) => {
      const i = D.kv[bucket].findIndex((e) => e[0] === key);
      return { bucket, key, payload: D.kv[bucket][i][1], revision: 4 + i * 3, operation: "PUT", created_ms: Date.now() - 3600000 };
    },
    kv_status: ({ bucket }) => ({ bucket, values: D.kv[bucket].length, bytes: 512, history: 5 }),
    kv_history: () => [],
    kv_put: () => 1,
    kv_create: () => null,
    kv_delete_key: () => null,
    kv_delete_bucket: () => null,
    kv_watch_subscribe: () => "kvw-" + sid++,
    obj_list: () => Object.keys(D.objs),
    obj_list_objects: ({ bucket }) => D.objs[bucket].map(([name, size], i) => ({ bucket, name, size, chunks: Math.ceil(size / 131072), digest: "SHA-256=" + "q3Fz7aB1xk9Lm0".repeat(3).slice(0, 43) + "=", modified_ms: Date.now() - (i + 1) * 86400000, deleted: false })),
    obj_info: ({ bucket, name }) => ({ bucket, name, size: 184320, chunks: 2 }),
    obj_get: () => "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Count 1 >>\nendobj\n% Invoice 2026-10-0413, Northwind, EUR 1284.50",
    obj_create: () => null,
    obj_delete: () => null,
    obj_delete_bucket: () => null,
    obj_put_payload: () => null,
    obj_upload: () => null,
    obj_download: () => null,
    sys_request: ({ subject }) => JSON.stringify(sys[subject] ?? { ok: true }),
    sys_subscribe: () => {
      let i = 0;
      const t = setInterval(() => {
        const [s, p] = D.sysEvents[i++ % D.sysEvents.length];
        window.__emit("sys-message", { subject: s, payload: JSON.stringify(p), timestamp_ms: Date.now() });
      }, 30);
      setTimeout(() => clearInterval(t), 400);
      return "sys-" + sid++;
    },
    decode_detect: ({ payload }) => { try { JSON.parse(payload); return "json"; } catch { return "utf8"; } },
    decode_format: ({ payload }) => { try { return JSON.stringify(JSON.parse(payload), null, 2); } catch { return payload; } },
    schema_message_names: () => [],
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

const SYSEV = [
  ["$SYS.ACCOUNT.SHOP.CONNECT", { client: { id: 4412, name: "orders-api-7", host: "10.20.4.52" } }],
  ["$SYS.ACCOUNT.SHOP.SERVER.CONNS", { conns: 214, leafnodes: 0 }],
  ["$SYS.SERVER.NDK7XQ2.STATSZ", { conns: 214, subs: 1840, mem: 41943040 }],
  ["$SYS.ACCOUNT.SHOP.DISCONNECT", { client: { id: 4390, name: "pick-station-4" }, reason: "Client Closed" }],
];

const SUBJECTS = [
  ["orders.created", 4120], ["orders.updated", 3388], ["orders.cancelled", 214], ["payments.authorized", 3870],
  ["payments.captured", 3702], ["inventory.reserved", 4410], ["inventory.low", 37], ["shipments.dispatched", 2951],
  ["shipments.delivered", 2760],
].map(([subject, count]) => ({ subject, message_count: count, last_update: T0 - 1200 }));

export const mock = `(${mockMain.toString()})(${JSON.stringify({ t0: T0, streams: STREAMS, msgs: MSGS, kv: KV, objs: OBJS, subjects: SUBJECTS, sysEvents: SYSEV })})`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function clickText(page, selector, text, nth = 0) {
  const ok = await page.evaluate(
    (selector, text, nth) => {
      const els = [...document.querySelectorAll(selector)].filter((e) => e.textContent.trim().startsWith(text));
      const el = els[nth];
      if (!el) return false;
      el.click();
      return true;
    },
    selector,
    text,
    nth,
  );
  if (!ok) throw new Error(`no element ${selector} with text ${text}`);
  await sleep(150);
}

async function typeInto(page, currentValue, text) {
  const ok = await page.evaluate((currentValue) => {
    const el = [...document.querySelectorAll("input, textarea")].find((e) => e.value === currentValue);
    if (!el) return false;
    el.focus();
    el.select();
    return true;
  }, currentValue);
  if (!ok) throw new Error(`no input with value ${currentValue}`);
  await page.keyboard.type(text, { delay: 0 });
  await sleep(100);
}

async function connectClient(page, switchMode = true) {
  if (switchMode) {
    await clickText(page, "button", "Client");
    await sleep(200);
  }
  await clickText(page, "button", "Connect");
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some((b) => b.textContent.includes("Disconnect")), { timeout: 5000 });
  await sleep(200);
}

async function feed(page, count) {
  await page.evaluate(async (count) => {
    const m = window.__msgs;
    for (let i = 0; i < count; i++) {
      const x = m[i % m.length];
      window.__emit("client-message", { ...x, timestamp_ms: Date.now() - (count - i) * 900 });
      await new Promise((r) => setTimeout(r, 4));
    }
  }, count);
  await sleep(300);
}

export const scenes = [
  {
    id: "server",
    headline: "Run a <em>NATS server</em> on your Mac",
    sub: "Start an embedded server in one click, then publish and watch every subject and message.",
    setup: async (page) => {
      await clickText(page, "button", "Start server");
      await sleep(1300);
    },
  },
  {
    id: "client",
    headline: "Watch <em>live traffic</em> on any subject",
    sub: "Connect to any server, subscribe with wildcards or queue groups, and inspect headers and JSON payloads.",
    setup: async (page) => {
      await clickText(page, "button", "Client");
      await sleep(200);
      await typeInto(page, "127.0.0.1", "10.20.4.11");
      await typeInto(page, "nats-explorer", "ops-console");
      await connectClient(page, false);
      await typeInto(page, "demo.>", "orders.>");
      await clickText(page, "button", "Subscribe");
      await typeInto(page, "orders.>", "payments.*");
      await clickText(page, "button", "Subscribe");
      await typeInto(page, "payments.*", "inventory.>");
      await clickText(page, "button", "Subscribe");
      await feed(page, 14);
    },
  },
  {
    id: "jetstream",
    headline: "See every <em>JetStream</em> stream",
    sub: "Message counts, sizes, subjects and consumers for each stream, at a glance.",
    setup: async (page) => {
      await connectClient(page);
      await clickText(page, "[role=tab]", "JetStream");
      await sleep(700);
    },
  },
  {
    id: "kv",
    headline: "Browse and edit <em>KV buckets</em>",
    sub: "Open any bucket, read keys with revisions, and update values while the live watch keeps the list current.",
    setup: async (page) => {
      await connectClient(page);
      await clickText(page, "[role=tab]", "KV");
      await sleep(900);
      await clickText(page, "main tbody tr", "feature.express", 0);
      await sleep(300);
    },
  },
  {
    id: "objects",
    headline: "Manage <em>object stores</em> too",
    sub: "List the files in a bucket with size, chunks and digest, then upload, download or delete them.",
    setup: async (page) => {
      await connectClient(page);
      await clickText(page, "[role=tab]", "Objects");
      await sleep(900);
      await clickText(page, "main tbody tr", "invoice-2026-10-0413", 0);
      await sleep(300);
    },
  },
];

/** Extra states for render-selects.mjs: selects the App Store scenes never show enabled. */
export const selectScenes = [
  {
    id: "client-form",
    async setup(page) {
      await clickText(page, "button", "Client");
      await sleep(300);
    },
  },
  {
    id: "schema",
    // The mock has no protobuf registry: answer the two schema reads the panel makes on mount.
    init: `const invoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = async (cmd, args) => (cmd === "schema_message_names" ? ["shop.Order", "shop.Payment", "shop.Shipment"] : cmd === "schema_bindings" ? {} : invoke(cmd, args));`,
    async setup(page) {
      await connectClient(page);
      await clickText(page, "[role=tab]", "Schema");
      await sleep(500);
    },
  },
];
