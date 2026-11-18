import assert from "node:assert/strict";
import * as fs from "node:fs";
import { describe, it } from "node:test";
import { CHECK_LINE_SOURCE, parseCheckOutput } from "../../src/checkOutput.ts";
import { failureMessages, findScenarios, parseTestReport, summarize } from "../../src/testReport.ts";

const fixture = (name: string) => fs.readFileSync(new URL(`../fixtures/${name}`, import.meta.url), "utf8");

describe("check output", () => {
  it("problemMatcher regexp in package.json matches the parser's", () => {
    const pkg = JSON.parse(fs.readFileSync(new URL("../../package.json", import.meta.url), "utf8"));
    const matcher = pkg.contributes.problemMatchers.find((m: { name: string }) => m.name === "voltrus-plc");
    assert.equal(matcher.pattern.regexp, CHECK_LINE_SOURCE);
  });

  it("parses real voltrus-plc check output", () => {
    // Captured from target/release/voltrus-plc check on a broken project.
    const { diagnostics, summary } = parseCheckOutput(fixture("check-output.txt"));
    assert.deepEqual(summary, { files: 2, errors: 3, warnings: 1 });
    assert.equal(diagnostics.length, 4);
    assert.deepEqual(diagnostics[0], {
      file: "Broken.st",
      line: 5,
      column: 7,
      severity: "error",
      message: "undefined identifier 'y'",
    });
    assert.deepEqual(diagnostics[2], {
      file: "Broken.st",
      line: 3,
      column: 2,
      severity: "warning",
      message: "variable 'x' may be uninitialized",
    });
    assert.deepEqual(diagnostics[3], {
      file: "sub/Parse.st",
      line: 1,
      column: 1,
      severity: "error",
      message: "invalid expression",
    });
  });

  it("handles Windows paths and CRLF", () => {
    const { diagnostics } = parseCheckOutput("sub\\A.st:2:3: error: error: boom at 1..2\r\n");
    assert.deepEqual(diagnostics, [{ file: "sub/A.st", line: 2, column: 3, severity: "error", message: "boom" }]);
  });
});

describe("test report", () => {
  const report = parseTestReport(fixture("test-report.json"));

  it("parses suites and cases from real --json output", () => {
    assert.equal(report.passed, false);
    assert.deepEqual(report.suites.map((s) => s.name), ["Bad.test.yaml", "sub/TankLevel.test.yaml"]);
    const tank = report.suites[1];
    assert.deepEqual(tank.cases.map((c) => c.outcome), ["passed", "failed", "failed"]);
    assert.match(summarize(report), /^1 passed, 3 failed \(2 files, \d+ ms\)$/);
  });

  it("collects failure messages, including load errors", () => {
    assert.deepEqual(failureMessages(report.suites[1].cases[1]), [
      "inlet_valve: expected FALSE, got TRUE (BOOL)",
      "level: expected 3, got 41.00 (REAL)",
    ]);
    assert.match(failureMessages(report.suites[0].cases[0])[0], /failed to read program/);
  });

  it("rejects output without a report", () => {
    assert.throws(() => parseTestReport("error: boom"), /no JSON/);
  });

  it("finds scenario names and lines in YAML", () => {
    const yaml = [
      "program: TankLevel.st",
      "scenarios:",
      "  - name: low level opens inlet # comment",
      "    steps:",
      "      - set: {pulse: true}",
      "        advance: 1scans",
      '  - name: "quoted: name"',
      "  -   name: 'it''s'",
    ].join("\n");
    assert.deepEqual(findScenarios(yaml), [
      { name: "low level opens inlet", line: 2 },
      { name: "quoted: name", line: 6 },
      { name: "it's", line: 7 },
    ]);
  });
});
