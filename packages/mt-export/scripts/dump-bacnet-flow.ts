// Writes the RustRED flow export of the BACnet simulator fixture, pointed at a local simulator, for
// validate-rustred-bacnet.sh: bun dump-bacnet-flow.ts <port>
//
// The export is used as it is, with write nodes on. Three things are added so a run checks more than the reads: the
// inject node of "write Valve" gets a payload and fires once after the reads started, and a COV subscription on the
// supply temperature feeds the debug node through a function that tags it.
import { exportTags } from "../src";
import { BACNET_SIM_LIST } from "../tests/fixtures";

const port = Number(process.argv[2]);
const list = { ...BACNET_SIM_LIST, source: { ...BACNET_SIM_LIST.source, port } };
const nodes = JSON.parse(exportTags("rustred", list, { bacnetWriteNodes: true }).content) as Record<string, any>[];

const tab = nodes[0].id;
const config = nodes.find((n) => n.type === "bacnet-config")!.id;
const debug = nodes.find((n) => n.type === "debug" && n.name === "values")!.id;

const writeInject = nodes.find((n) => n.type === "inject" && n.name === "write Valve")!;
Object.assign(writeInject, { payload: "42", payloadType: "num", once: true, onceDelay: 2 });

nodes.push(
  {
    id: "c0c0c0c0c0c0c0c1", type: "bacnet read", z: tab, name: "Supply temp COV", configNode: config, objectType: "analogInput", objectInstance: 1,
    property: "presentValue", subscribeCov: true, covLifetime: 60, x: 400, y: 600, wires: [["c0c0c0c0c0c0c0c2"]],
  },
  {
    id: "c0c0c0c0c0c0c0c2", type: "function", z: tab, name: "tag COV", outputs: 1, timeout: 0, noerr: 0, initialize: "", finalize: "", libs: [],
    func: 'msg.payload = { ["COV " + msg.bacnet.cov]: msg.payload };\nreturn msg;', x: 650, y: 600, wires: [[debug]],
  },
);
process.stdout.write(JSON.stringify(nodes, null, 2) + "\n");
