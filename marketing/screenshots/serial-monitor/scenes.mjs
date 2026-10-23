// App Store scenes for Serial Monitor. Rendered by marketing/screenshots/render.mjs.
//
// The mock stands in for the Rust side of the app (apps/serial-monitor/src-tauri/src/main.rs):
//   list_available_ports -> PortInfo[], connect/disconnect, send({data, format}) and the "serial-data" event
//   ({ hex, bytes, timestamp }). The mocked device is a Modbus RTU energy meter at slave address 1 (FC03 and FC06
//   with real CRC-16) that also answers a few AT commands, so what the screenshots show is what a real port returns.

import { pickOption } from "../pick.mjs";

function installMock() {
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

  const crc16 = (bytes) => {
    let crc = 0xffff;
    for (const b of bytes) {
      crc ^= b;
      for (let i = 0; i < 8; i++) crc = crc & 1 ? (crc >> 1) ^ 0xa001 : crc >> 1;
    }
    return [crc & 0xff, crc >> 8];
  };
  const withCrc = (bytes) => [...bytes, ...crc16(bytes)];
  const hex = (bytes) => bytes.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join(" ");

  // Holding registers 0..: voltage x10, current x100, power W, frequency x100, PF x1000, energy (kWh) hi/lo, temperature x10 ...
  const regs = [2304, 2311, 2298, 1425, 1388, 1461, 9840, 9712, 10120, 5001, 962, 0, 15873, 385, 0, 0];
  const word = (n) => [(n >> 8) & 0xff, n & 0xff];

  function respond(req) {
    if (req.length >= 8 && req[0] === 1 && req[1] === 3) {
      const addr = (req[2] << 8) | req[3];
      const qty = (req[4] << 8) | req[5];
      const body = [];
      for (let i = 0; i < qty; i++) body.push(...word(regs[addr + i] ?? 0));
      return withCrc([1, 3, body.length, ...body]);
    }
    if (req.length >= 8 && req[0] === 1 && req[1] === 6) {
      regs[(req[2] << 8) | req[3]] = (req[4] << 8) | req[5];
      return req.slice(0, 8);
    }
    const text = String.fromCharCode(...req).trim();
    if (text === "AT") return [...new TextEncoder().encode("\r\nOK\r\n")];
    if (text === "AT+CSQ") return [...new TextEncoder().encode("\r\n+CSQ: 21,0\r\n\r\nOK\r\n")];
    if (text === "ATI") return [...new TextEncoder().encode("\r\nQuectel\r\nEC25\r\n\r\nOK\r\n")];
    return null;
  }

  let connected = false;
  const decode = (data, format) => {
    if (format === "hex") return data.split(/\s+/).filter(Boolean).map((h) => parseInt(h, 16));
    return [...new TextEncoder().encode(data)];
  };

  const commands = {
    store_status: () => window.__MOCK_STATUS || { store: true, pro: true, reason: "purchased", price: null },
    open_url: () => null,
    list_available_ports: () => [
      { port_name: "/dev/cu.usbserial-A50285BI", port_type: "USB", manufacturer: "FTDI", product: "FT232R USB UART", serial_number: "A50285BI" },
      { port_name: "/dev/cu.usbmodem14101", port_type: "USB", manufacturer: "Arduino LLC", product: "Arduino Uno", serial_number: "85735313731351E08181" },
      { port_name: "/dev/cu.SLAB_USBtoUART", port_type: "USB", manufacturer: "Silicon Labs", product: "CP2102 USB to UART Bridge", serial_number: "0001" },
      { port_name: "/dev/cu.Bluetooth-Incoming-Port", port_type: "Bluetooth", manufacturer: null, product: null, serial_number: null },
    ],
    connect: () => { connected = true; return null; },
    disconnect: () => { connected = false; return null; },
    send: ({ data, format }) => {
      if (!connected) throw "Port is not open";
      const bytes = decode(data, format);
      const reply = respond(bytes);
      if (reply) {
        const wire = Math.round((reply.length * 11 * 1000) / 19200) + 14;
        setTimeout(() => emit("serial-data", { hex: hex(reply), bytes: reply, timestamp: Date.now() }), wire);
      }
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
      throw `unknown command ${cmd}`;
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

// ── Shared scene plumbing ────────────────────────────────────────────────────────────────────────

const profiles = [
  { name: "Energy meter RS-485", port_name: "/dev/cu.usbserial-A50285BI", baud_rate: 19200, data_bits: 8, stop_bits: 1, parity: "even", timeout_ms: 1000 },
];
const macros = [
  { name: "Read voltages", data: "01 03 00 00 00 03 05 CB", format: "hex" },
  { name: "Read power", data: "01 03 00 06 00 03 E4 0A", format: "hex" },
  { name: "Modem signal", data: "AT+CSQ", format: "ascii" },
];

const initCommon = `
  localStorage.setItem("serial.profiles", ${JSON.stringify(JSON.stringify(profiles))});
  localStorage.setItem("serial.macros", ${JSON.stringify(JSON.stringify(macros))});
`;

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

async function setSendMode(page, label) {
  await clickButton(page, label);
  await sleep(100);
}

async function typeSend(page, text) {
  const ta = await page.$("textarea.vt-textarea");
  await ta.click({ clickCount: 3 });
  await page.keyboard.press("Backspace");
  await ta.type(text, { delay: 0 });
  await clickButton(page, "Send");
}

async function connect(page, port, baud, parity) {
  const showing = await page.evaluate(() => [...document.querySelectorAll("button.vt-select-trigger")].map((b) => b.textContent.trim()).find((t) => t.startsWith("/dev/") || t === "Select port..."));
  await pickOption(page, showing, port);
  if (baud) await pickOption(page, "115200", String(baud));
  if (parity) await pickOption(page, "None", parity[0].toUpperCase() + parity.slice(1));
  await sleep(200);
  await clickButton(page, "Connect");
  await sleep(500);
}

const MODBUS_READS = ["01 03 00 00 00 03 05 CB", "01 03 00 06 00 03 E4 0A", "01 03 00 09 00 04 15 CB", "01 03 00 0C 00 02 04 08"];

export const scenes = [
  {
    id: "talk-to-device",
    headline: "Talk to any <em>serial device</em>",
    sub: "Open a port, set baud rate, parity and stop bits, and watch every byte in and out.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 0.88);
      await connect(page, "/dev/cu.usbserial-A50285BI", 19200, "even");
      await setSendMode(page, "HEX");
      for (const frame of MODBUS_READS) {
        await typeSend(page, frame);
        await sleep(450);
      }
      await sleep(300);
    },
  },
  {
    id: "hex-and-ascii",
    headline: "Send <em>HEX or text</em>, read the reply",
    sub: "Switch between raw bytes and ASCII commands. Every response lands in the table right away.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 0.88);
      await connect(page, "/dev/cu.SLAB_USBtoUART", 115200);
      await setSendMode(page, "ASCII");
      await typeSend(page, "AT");
      await sleep(400);
      await typeSend(page, "AT+CSQ");
      await sleep(400);
      await typeSend(page, "ATI");
      await sleep(500);
      await page.type("textarea.vt-textarea", "AT+CSQ");
      await sleep(200);
    },
  },
  {
    id: "no-hardware",
    headline: "Try it <em>without hardware</em>",
    sub: "Replay a Modbus RTU master or slave conversation to see how traffic and the log look.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 0.88);
      await pickOption(page, "Select template...", "Modbus RTU Master");
      await clickButton(page, "Start simulation");
      await sleep(12500);
    },
  },
  {
    id: "profiles-macros",
    headline: "Save your <em>setup and commands</em>",
    sub: "Keep port settings per device and one-click macros for the frames you send all day.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 0.88);
      await pickOption(page, "Select profile...", "Energy meter RS-485");
      await clickButton(page, "Connect");
      await sleep(400);
      await setSendMode(page, "HEX");
      await pickOption(page, "Send macro", "Read voltages");
      await clickButton(page, "Send");
      await sleep(400);
      await pickOption(page, "Send macro", "Read power");
      await clickButton(page, "Send");
      await sleep(400);
      await pickOption(page, "Send macro", "Read voltages");
      await sleep(300);
    },
  },
  {
    id: "log",
    headline: "A full <em>communication log</em>",
    sub: "Timestamps, direction and payload for every frame. Filter it, copy lines or export it as CSV.",
    init: initCommon,
    settleMs: 400,
    async setup(page) {
      await prepare(page, 0.88);
      await connect(page, "/dev/cu.usbserial-A50285BI", 19200, "even");
      await setSendMode(page, "HEX");
      for (let k = 0; k < 3; k++) {
        for (const frame of MODBUS_READS) {
          await typeSend(page, frame);
          await sleep(260);
        }
      }
      await typeSend(page, "01 06 00 0B 00 01 F9 C9");
      await sleep(400);
    },
  },
];
