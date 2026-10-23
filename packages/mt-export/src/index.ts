import type { ExportFormat, ExportResult, FormatId, Protocol, TagList } from "./model";
import { csvFormat } from "./formats/csv";
import { jsonFormat } from "./formats/json";
import { ignitionFormat } from "./formats/ignition";
import { kepwareFormat } from "./formats/kepware";
import { noderedFormat, rustredFormat } from "./formats/nodered";
import { homeAssistantFormat } from "./formats/homeassistant";
import { voltrusGatewayFormat, voltrusScadaFormat } from "./formats/voltrus";

export * from "./model";
export { READ_FUNCTION, reference6 } from "./modbusAddress";
export { groupReads } from "./formats/nodered";

/** Every format, in the order the picker shows them. */
export const FORMATS: ExportFormat[] = [
  csvFormat,
  jsonFormat,
  ignitionFormat,
  kepwareFormat,
  rustredFormat,
  noderedFormat,
  homeAssistantFormat,
  voltrusScadaFormat,
  voltrusGatewayFormat,
];

export const formatsFor = (protocol: Protocol): ExportFormat[] => FORMATS.filter((f) => f.protocols.includes(protocol));

export function exportTags(id: FormatId, list: TagList): ExportResult {
  const format = FORMATS.find((f) => f.id === id);
  if (!format) throw new Error(`Unknown export format '${id}'`);
  if (!format.protocols.includes(list.protocol)) throw new Error(`${format.label} does not support ${list.protocol} tags`);
  return format.export(list);
}
