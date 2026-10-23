// Writes the RustRED flow export of the BACnet simulator fixture, pointed at a local simulator: bun dump-bacnet-flow.ts <port>
import { exportTags } from "../src";
import { BACNET_SIM_LIST } from "../tests/fixtures";

const port = Number(process.argv[2]);
const list = { ...BACNET_SIM_LIST, source: { ...BACNET_SIM_LIST.source, port } };
process.stdout.write(exportTags("rustred", list).content);
