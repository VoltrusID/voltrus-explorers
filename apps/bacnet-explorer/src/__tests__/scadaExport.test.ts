import { describe, expect, it } from "vitest";
import { exportTags } from "@mactools/export";
import { buildTagList } from "../scadaExport";
import type { ObjectListRow } from "../objectCsv";
import type { BacnetConfig, BacnetObjectDetail } from "../types";

const DEVICE: BacnetConfig = { id: "d", ip: "10.1.1.9", port: 47808, device_id: 2001 };

const detail = (over: Partial<BacnetObjectDetail>): BacnetObjectDetail => ({
  object_type: "analogInput", object_instance: 0, object_name: null, description: null, present_value: null, units: null,
  units_symbol: null, status_flags: null, reliability: null, out_of_service: null, properties: [], ...over,
});

const ROWS: ObjectListRow[] = [
  { object: { object_type: "analogInput", object_instance: 1, object_name: null }, detail: detail({ object_name: "Zone Temp", units_symbol: "°C", description: "Zone temperature" }) },
  { object: { object_type: "binaryOutput", object_instance: 2, object_name: "Fan" }, detail: null },
  { object: { object_type: "analogValue", object_instance: 3, object_name: null }, detail: detail({ object_name: "Zone Temp" }) },
  { object: { object_type: "schedule", object_instance: 1, object_name: "Weekly" }, detail: null },
  { object: { object_type: "multiStateValue", object_instance: 4, object_name: null }, detail: null },
];

describe("buildTagList", () => {
  const { list, skipped } = buildTagList(ROWS, DEVICE, "AHU-2");

  it("keeps the nine supported object types and counts the rest", () => {
    expect(list.tags).toHaveLength(4);
    expect(skipped).toBe(1);
    expect(list.source).toEqual({ name: "AHU-2", host: "10.1.1.9", port: 47808, deviceId: 2001, pollMs: 5000 });
  });

  it("names tags after the object, falls back to type_instance, and splits duplicates by instance", () => {
    expect(list.tags.map((t) => t.name)).toEqual(["Zone Temp_1", "Fan", "Zone Temp_3", "multiStateValue_4"]);
  });

  it("carries units, descriptions and writability", () => {
    expect(list.tags[0]).toMatchObject({ unit: "°C", description: "Zone temperature", writable: false, property: "presentValue" });
    expect(list.tags[1].writable).toBe(true);
    expect(list.tags[2].writable).toBe(true);
  });

  it("feeds every BACnet exporter", () => {
    for (const id of ["csv", "json", "ignition", "voltrus-scada", "voltrus-gateway"] as const) {
      expect(exportTags(id, list).content.length).toBeGreaterThan(0);
    }
  });
});
