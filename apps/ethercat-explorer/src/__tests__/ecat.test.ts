import { describe, expect, it } from "vitest";
import { devicesCsv, ds402State, entryNumber, entryValue, extractBits, interfaceLabel, parseAddr, pcap } from "../ecat";
import type { DeviceInfo } from "../types";

describe("process data entries", () => {
  // EL3102 channel 2 as the simulator maps it: 32 bits of channel 1, then status bits, then the 16-bit value.
  const img = [0, 0, 0, 0, 0b0000_0010, 0, 0x00, 0x80];

  it("extracts bits little endian at any offset", () => {
    expect(extractBits(img, 33, 1)).toBe(1n);
    expect(extractBits(img, 48, 16)).toBe(0x8000n);
    expect(extractBits([0xff], 4, 8)).toBe(0x0fn);
  });

  it("formats by data type", () => {
    expect(entryValue({ dataType: 0x0003, bits: 16 }, 0x8000n)).toBe("-32768");
    expect(entryValue({ dataType: 0x0006, bits: 16 }, 0x8000n)).toBe("32768");
    expect(entryValue({ dataType: 0x0001, bits: 1 }, 1n)).toBe("TRUE");
    expect(entryValue({ dataType: 0x0031, bits: 2 }, 2n)).toBe("0b10");
    expect(entryValue({ dataType: 0x0004, bits: 32 }, 0xffffffffn)).toBe("-1");
    const f = new DataView(new ArrayBuffer(4));
    f.setFloat32(0, 1.5, true);
    expect(entryValue({ dataType: 0x0008, bits: 32 }, BigInt(f.getUint32(0, true)))).toBe("1.5");
    expect(entryNumber({ dataType: 0x0001, bits: 1 }, 0n)).toBe(0);
    expect(entryNumber({ dataType: 0x001e, bits: 8 }, 0x1fn)).toBe(31);
  });

  it("names CiA 402 states", () => {
    expect(ds402State(0x0250)).toBe("Switch on disabled");
    expect(ds402State(0x0231)).toBe("Ready to switch on");
    expect(ds402State(0x1637)).toBe("Operation enabled");
    expect(ds402State(0x0218)).toBe("Fault");
  });
});

describe("files", () => {
  it("writes a pcap Wireshark reads", () => {
    const f = pcap([{ tsUs: 1_500_000, out: true, datagrams: [], bytes: [1, 2, 3] }], 1_000_000);
    const dv = new DataView(f.buffer);
    expect(dv.getUint32(0, true)).toBe(0xa1b2c3d4);
    expect(dv.getUint32(20, true)).toBe(1);
    expect(dv.getUint32(24, true)).toBe(1001);
    expect(dv.getUint32(28, true)).toBe(500_000);
    expect(dv.getUint32(32, true)).toBe(3);
    expect(Array.from(f.slice(40))).toEqual([1, 2, 3]);
  });

  it("exports devices as CSV with quoting", () => {
    const d = { position: 0, address: 0x1000, alias: 0, name: "EL3102", description: "EL3102 2Ch. Ana. Input +/-10V, Diff.", vendorId: 2, vendorName: "Beckhoff Automation",
      productCode: 0x0c1e3052, revision: 0x00140000, serial: 7, escType: 0x12, escName: "ET1200", dc: "64-bit", mailbox: ["CoE"], state: "OP", alText: null } as unknown as DeviceInfo;
    const csv = devicesCsv([d]).split("\n");
    expect(csv[1]).toBe('0,0x1000,0,EL3102,"EL3102 2Ch. Ana. Input +/-10V, Diff.",0x00000002,Beckhoff Automation,0x0C1E3052,0x00140000,7,ET1200,64-bit,CoE,OP,');
  });

  it("parses register addresses and names interfaces", () => {
    expect(parseAddr("0x0130")).toBe(0x130);
    expect(parseAddr("120")).toBe(0x120);
    expect(parseAddr("zz")).toBeNull();
    expect(interfaceLabel({ name: "en5", label: "USB 10/100/1000 LAN", mac: "00:e0:4c:68:01:02", up: true, running: false, wireless: false }))
      .toBe("en5 · USB 10/100/1000 LAN · 00:e0:4c:68:01:02 · no link");
  });
});
