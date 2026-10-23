// Writes the flow export of the shared test fixture, pointed at a local simulator: bun dump-flow.ts <format> <wordOrder> <port>
import { exportTags, type FormatId, type WordOrder } from "../src";
import { MODBUS_LIST } from "../tests/fixtures";

const [id, order, port] = process.argv.slice(2);
const list = { ...MODBUS_LIST, source: { ...MODBUS_LIST.source, host: "127.0.0.1", port: Number(port), wordOrder: order as WordOrder, pollMs: 1000 } };
process.stdout.write(exportTags(id as FormatId, list).content);
