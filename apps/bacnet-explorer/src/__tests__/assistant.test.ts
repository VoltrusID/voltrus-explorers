import { describe, expect, it } from "vitest";
import { bacnetConfig, bacnetError } from "../assistantAdapter";

describe("bacnetError", () => {
  it("recognises a connection with no BACnet answer as a timeout", () => {
    expect(bacnetError("Connection failed (no BACnet response within 3s).")?.kind).toBe("timeout");
    expect(bacnetError("Read timed out")?.kind).toBe("timeout");
  });

  it("leaves other failures as other and nothing as nothing", () => {
    expect(bacnetError("Abort: segmentation not supported")?.kind).toBe("other");
    expect(bacnetError(null)).toBeUndefined();
    expect(bacnetError("")).toBeUndefined();
  });

  it("caps the message", () => {
    expect(bacnetError("x".repeat(1000))?.message).toHaveLength(300);
  });
});

describe("bacnetConfig", () => {
  it("sends the device instance and port, never the address", () => {
    const cfg = bacnetConfig({ device: { port: 47808, device_id: 1001 }, openDevices: 2 });
    expect(cfg).toEqual({ device_instance: 1001, udp_port: 47808, open_devices: 2 });
    expect(JSON.stringify(cfg)).not.toMatch(/\d+\.\d+\.\d+\.\d+/);
  });

  it("is just the count when nothing is open", () => {
    expect(bacnetConfig({ device: null, openDevices: 0 })).toEqual({ open_devices: 0 });
  });
});
