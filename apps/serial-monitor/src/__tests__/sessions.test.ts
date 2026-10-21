import { describe, it, expect } from "vitest";
import {
  TRAFFIC_ROWS, appendTraffic, configSummary, defaultConfig, newSession, pickFreePort, shortPortName,
} from "../sessions";
import type { PortInfo, TrafficEntry } from "../types";

const port = (port_name: string): PortInfo => ({
  port_name, port_type: "usb", manufacturer: null, product: null, serial_number: null,
});

describe("shortPortName", () => {
  it("strips the macOS and Linux device prefixes", () => {
    expect(shortPortName("/dev/cu.usbserial-1410")).toBe("usbserial-1410");
    expect(shortPortName("/dev/ttyUSB0")).toBe("ttyUSB0");
  });
  it("keeps Windows names and names an empty selection", () => {
    expect(shortPortName("COM3")).toBe("COM3");
    expect(shortPortName("")).toBe("No port");
  });
});

describe("configSummary", () => {
  it("formats baud, data bits, parity and stop bits", () => {
    expect(configSummary(defaultConfig())).toBe("115200 8N1");
    expect(configSummary({ ...defaultConfig(), baud_rate: 9600, data_bits: 7, parity: "even", stop_bits: 2 })).toBe("9600 7E2");
    expect(configSummary({ ...defaultConfig(), parity: "odd" })).toBe("115200 8O1");
  });
});

describe("pickFreePort", () => {
  const ports = [port("A"), port("B"), port("C")];
  it("skips ports other sessions already picked", () => {
    const sessions = [newSession("s1", defaultConfig("A")), newSession("s2", defaultConfig("B"))];
    expect(pickFreePort(ports, sessions)).toBe("C");
  });
  it("returns an empty name when every port is taken", () => {
    const sessions = ports.map((p, i) => newSession(`s${i}`, defaultConfig(p.port_name)));
    expect(pickFreePort(ports, sessions)).toBe("");
  });
});

describe("appendTraffic", () => {
  it("keeps the newest rows only", () => {
    let rows: TrafficEntry[] = [];
    for (let i = 0; i < TRAFFIC_ROWS + 50; i++) {
      rows = appendTraffic(rows, { direction: "RX", data: String(i), timestamp: "" });
    }
    expect(rows.length).toBe(TRAFFIC_ROWS + 1);
    expect(rows[rows.length - 1].data).toBe(String(TRAFFIC_ROWS + 49));
  });
});

describe("newSession", () => {
  it("starts closed with empty traffic", () => {
    const s = newSession("s9", defaultConfig("X"));
    expect(s).toMatchObject({ id: "s9", connected: false, connecting: false, traffic: [], repeating: false });
  });
});
