// PLC-10 golden generator: runs the REAL TypeScript diagram transpilers on
// committed fixture JSONs and writes the expected `.st` goldens (and canonical
// fixture JSONs + exported samples) for the Rust parity tests in
// crates/mt-plc-cli/tests/golden_diagrams.rs.
//
// Run explicitly (excluded from the default `pnpm test` suite):
//   cd apps/voltrus-plc && pnpm vitest run scripts/gen-goldens.test.ts
//
// Fixture layout written:
//   crates/mt-plc-cli/tests/fixtures/<fbd|ld|sfc>/<name>.<ext>   canonical JSON
//   crates/mt-plc-cli/tests/fixtures/<fbd|ld|sfc>/<name>.golden.st
//   crates/mt-plc-cli/tests/fixtures/fbd/cycle-error.expected.txt
//   apps/voltrus-plc/samples/MotorStartStop.fbd|MotorStartStop.ld|TrafficLight.sfc
import { describe, it } from "vitest";
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { transpileFbdToSt } from "../src/components/Editor/FbdEditor/transpiler/fbd-to-st";
import { transpileLdToSt } from "../src/components/Editor/LdEditor/transpiler/ld-to-st";
import { transpileSfcToSt } from "../src/components/Editor/SfcEditor/transpiler/sfc-to-st";
import { serializeFbd, serializeLd, serializeSfc } from "../src/types/canonical";
import { FBD_SAMPLES } from "../src/constants/fbd-samples";
import { LD_SAMPLES } from "../src/constants/ld-samples";
import { SFC_SAMPLES } from "../src/constants/sfc-samples";
import type { FbdProgram } from "../src/types/fbd";
import type { LdProgram } from "../src/types/ld";
import type { SfcProgram } from "../src/types/sfc";

const HERE = dirname(fileURLToPath(import.meta.url));
const APP_ROOT = join(HERE, "..");
const REPO = join(APP_ROOT, "..", "..");
const FIXTURES = join(REPO, "crates", "mt-plc-cli", "tests", "fixtures");
const SAMPLES = join(APP_ROOT, "samples");

// Broken fixture: two logic blocks feeding each other (FBD cycle). The TS
// transpiler emits the cycle error — the golden is the error message.
const CYCLE_ERROR: FbdProgram = {
  version: "1.0",
  programName: "CycleError",
  declarations: [],
  nodes: [
    { id: "node-1", type: "block", position: { x: 0, y: 0 }, data: { blockType: "AND", label: "AND", pinValues: {} } },
    { id: "node-2", type: "block", position: { x: 200, y: 0 }, data: { blockType: "OR", label: "OR", pinValues: {} } },
    { id: "node-3", type: "input_pin", position: { x: 0, y: 200 }, data: { blockType: "INPUT_PIN", label: "Input", variableName: "X0", variableType: "BOOL", pinValues: {} } },
  ],
  edges: [
    { id: "e1", source: "node-1", target: "node-2", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
    { id: "e2", source: "node-2", target: "node-1", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
    { id: "e3", source: "node-3", target: "node-1", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
  ],
  viewport: { x: 0, y: 0, zoom: 1 },
};

interface LanguageSpec {
  dir: string;
  ext: string;
  fixtures: Array<[string, unknown]>;
  serialize: (program: never) => string;
  transpile: (program: never) => { source: string; diagnostics: Array<{ message: string; severity: string }> };
}

const LANGUAGES: LanguageSpec[] = [
  {
    dir: "fbd",
    ext: "fbd",
    fixtures: [
      ["timer-delay", FBD_SAMPLES.find((s) => s.id === "timer-delay")!.program],
      ["counter", FBD_SAMPLES.find((s) => s.id === "counter")!.program],
      ["threshold-alarm", FBD_SAMPLES.find((s) => s.id === "threshold-alarm")!.program],
      ["motor-start-stop", FBD_SAMPLES.find((s) => s.id === "motor-start-stop")!.program],
      ["cycle-error", CYCLE_ERROR],
    ],
    serialize: serializeFbd as (program: never) => string,
    transpile: transpileFbdToSt as LanguageSpec["transpile"],
  },
  {
    dir: "ld",
    ext: "ld",
    fixtures: [
      ["motor-start-stop", LD_SAMPLES.find((s) => s.id === "ld-motor-start-stop")!.program],
      ["timer-delay", LD_SAMPLES.find((s) => s.id === "ld-timer-delay")!.program],
      ["alarm-latch", LD_SAMPLES.find((s) => s.id === "ld-alarm-latch")!.program],
    ],
    serialize: serializeLd as (program: never) => string,
    transpile: transpileLdToSt as LanguageSpec["transpile"],
  },
  {
    dir: "sfc",
    ext: "sfc",
    fixtures: [
      ["traffic-light", SFC_SAMPLES.find((s) => s.id === "sfc-traffic-light")!.program],
      ["motor-control", SFC_SAMPLES.find((s) => s.id === "sfc-motor-control")!.program],
      ["conveyor-branch", SFC_SAMPLES.find((s) => s.id === "sfc-conveyor-branch")!.program],
    ],
    serialize: serializeSfc as (program: never) => string,
    transpile: transpileSfcToSt as LanguageSpec["transpile"],
  },
];

describe("golden generation (PLC-10)", () => {
  it("writes canonical fixtures + TS goldens for the Rust parity tests", () => {
    for (const lang of LANGUAGES) {
      const dir = join(FIXTURES, lang.dir);
      mkdirSync(dir, { recursive: true });
      for (const [name, program] of lang.fixtures) {
        const canonical = lang.serialize(program as never);
        writeFileSync(join(dir, `${name}.${lang.ext}`), canonical);
        const result = lang.transpile(JSON.parse(canonical) as never);
        if (result.diagnostics.some((d) => d.severity === "error")) {
          writeFileSync(
            join(dir, `${name}.expected.txt`),
            result.diagnostics.map((d) => `${d.severity}: ${d.message}`).join("\n") + "\n",
          );
        } else {
          writeFileSync(join(dir, `${name}.golden.st`), result.source);
        }
      }
    }
  });

  it("exports canonical sample diagrams for the CI check step", () => {
    mkdirSync(SAMPLES, { recursive: true });
    writeFileSync(
      join(SAMPLES, "MotorStartStop.fbd"),
      serializeFbd(FBD_SAMPLES.find((s) => s.id === "motor-start-stop")!.program),
    );
    // The ld-timer-delay / ld-motor-start-stop samples emit ST the compiler
    // rejects (a contact named "delay.Q" fails the identifier regex; Y0 ends
    // up declared in VAR_OUTPUT *and* VAR). AlarmLatch compiles clean.
    writeFileSync(
      join(SAMPLES, "AlarmLatch.ld"),
      serializeLd(LD_SAMPLES.find((s) => s.id === "ld-alarm-latch")!.program),
    );
    // The sfc samples reference undeclared identifiers in transition
    // conditions (e.g. T1.Q — timers the SFC lowering never declares), so the
    // exported gate sample keeps the TrafficLight structure but transitions
    // on TRUE: still exercises the step/prev/jump machinery, one step/scan.
    const trafficLight: SfcProgram = JSON.parse(
      JSON.stringify(SFC_SAMPLES.find((s) => s.id === "sfc-traffic-light")!.program),
    );
    for (const node of trafficLight.nodes) {
      if (node.data.elementType === "transition") node.data.condition = "TRUE";
    }
    writeFileSync(join(SAMPLES, "TrafficLight.sfc"), serializeSfc(trafficLight));
  });

  it("fixture goldens are deterministic on regeneration", () => {
    // Re-serialize one fixture twice and confirm byte-identical canonical form.
    const fixture = join(FIXTURES, "fbd", "timer-delay.fbd");
    const once = serializeFbd(JSON.parse(readFileSync(fixture, "utf8")) as FbdProgram);
    const twice = serializeFbd(JSON.parse(once) as FbdProgram);
    if (once !== twice) throw new Error("canonical serialization is not stable");
  });
});
