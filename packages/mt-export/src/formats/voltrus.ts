import type { ExportFormat, ModbusTag, TagList } from "../model";
import { fileStem, slug, toJson } from "../util";

/**
 * Voltrus SCADA device file. For Modbus it is the body of `POST /api/v1/modbus-devices` in the voltrus server
 * (src/api/modbus_devices.rs `create_modbus_device`, `ModbusRegisterInput` in src/models/protocol_device_types.rs):
 * `id`, `name`, `ip`, `port`, `unit_id`, `interval_ms`, `endianness` (big | little | swapped) and `registers`, each with
 * `name`, `address` (u16), `register_type` (int16 | uint16 | int32 | float32 | float64), `register_area`
 * (holding | input | coil | discrete) and `scale`. The Modbus devices page of the SCADA imports this file by drag and drop.
 *
 * `RegisterConfig::protocol_address` (src/models/config_types.rs) treats an address of 30001 or more as a 5-digit reference
 * and subtracts 30001 or 40001, so a 0-based address from 30001 up cannot be stored; those tags are left out.
 *
 * BACnet lists use the two calls the BACnet devices page makes: the file is a `CreateBacnetDeviceRequest` (`name`, `ip`,
 * `port`, `device_instance`, `interval_ms`) plus `objects`, each a `CreateBacnetObjectRequest` (`object_type` as analogInput
 * and so on, `object_instance`, `property`, `name`) for `POST /api/v1/bacnet/devices/{id}/objects` (src/models/bacnet_types.rs).
 */

const MAX_PLAIN_ADDRESS = 30000;

export const voltrusScadaFormat: ExportFormat = {
  id: "voltrus-scada",
  label: "Voltrus SCADA device (JSON)",
  hint: "A device with its tags, for the SCADA's drag and drop import",
  extension: "json",
  protocols: ["modbus", "bacnet"],
  export(list: TagList) {
    const warnings: string[] = [];
    if (list.protocol === "bacnet") {
      const s = list.source;
      const body = {
        name: s.name,
        ip: s.host,
        port: s.port,
        device_instance: s.deviceId,
        interval_ms: s.pollMs,
        objects: list.tags.map((t) => ({ name: t.name, object_type: t.objectType, object_instance: t.instance, property: t.property })),
      };
      return { fileName: `${fileStem(s.name)}-voltrus-bacnet-device.json`, content: toJson(body), warnings };
    }

    const s = list.source;
    const registers = list.tags.flatMap((t: ModbusTag) => {
      if (t.address > MAX_PLAIN_ADDRESS) {
        warnings.push(`${t.name}: address ${t.address} is above ${MAX_PLAIN_ADDRESS}, which the SCADA reads as a 5-digit reference. Left out.`);
        return [];
      }
      return [{ name: t.name, address: t.address, register_type: t.type === "bool" ? "uint16" : t.type, register_area: t.area, scale: t.scale }];
    });
    if (s.transport === "rtu") warnings.push("The SCADA polls Modbus TCP only. Put the IP address of a serial gateway in the device after import.");
    const body = { id: slug(s.name, "modbus_device"), name: s.name, ip: s.host, port: s.port, unit_id: s.unitId, interval_ms: s.pollMs, endianness: s.wordOrder, registers };
    return { fileName: `${fileStem(s.name)}-voltrus-device.json`, content: toJson(body), warnings };
  },
};

/**
 * Voltrus Gateway (gRPC, MacTools/gateway/proto/gateway.proto). The gateway has no import file: a client sends a
 * `ConnectRequest` and then `ReadTagsRequest` or `SubscribeRequest` with a list of `TagAddress`. This file holds that
 * request data in the proto3 JSON mapping, so each part can be passed to `grpcurl -d @`:
 * `connect` is a ConnectRequest, `tags` is the `tags` field of ReadTagsRequest and SubscribeRequest, and `intervalMs`
 * is the SubscribeRequest field. Messages: ModbusConnectConfig, ModbusTagAddress (name, address, regType, scale,
 * registerArea) and BacnetConnectConfig / BacnetTagAddress (name, objectType, objectInstance, property).
 */
export const voltrusGatewayFormat: ExportFormat = {
  id: "voltrus-gateway",
  label: "Voltrus Gateway (JSON)",
  hint: "Connect request and tag addresses for the gateway's gRPC API",
  extension: "json",
  protocols: ["modbus", "bacnet"],
  export(list: TagList) {
    const warnings: string[] = [];
    if (list.protocol === "bacnet") {
      const s = list.source;
      const body = {
        connect: { bacnet: { id: slug(s.name, "bacnet_device"), ip: s.host, port: s.port, deviceId: s.deviceId, pollIntervalMs: s.pollMs } },
        intervalMs: s.pollMs,
        tags: list.tags.map((t) => ({ bacnet: { name: t.name, objectType: t.objectType, objectInstance: t.instance, property: t.property } })),
      };
      return { fileName: `${fileStem(s.name)}-gateway.json`, content: toJson(body), warnings };
    }
    const s = list.source;
    if (s.transport === "rtu") warnings.push("The gateway's Modbus adapter connects over TCP only. Use the IP address of a serial gateway.");
    const tags = list.tags.map((t) => ({
      modbus: { name: t.name, address: t.address, regType: t.type === "bool" ? "uint16" : t.type, scale: t.scale, registerArea: t.area },
    }));
    const body = {
      connect: { modbus: { id: slug(s.name, "modbus_device"), ip: s.host, port: s.port, unitId: s.unitId, intervalMs: s.pollMs, endianness: s.wordOrder } },
      intervalMs: s.pollMs,
      tags,
    };
    return { fileName: `${fileStem(s.name)}-gateway.json`, content: toJson(body), warnings };
  },
};
