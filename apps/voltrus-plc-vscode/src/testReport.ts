// Types and helpers for `voltrus-plc test --json`.

export interface AssertionReport {
  name: string;
  passed: boolean;
  expected: string;
  actual: string;
}

export interface CaseReport {
  name: string;
  outcome: "passed" | "failed" | "error";
  duration_ms: number;
  failures: string[];
  error: string | null;
  assertions: AssertionReport[];
}

export interface SuiteReport {
  /** YAML path relative to the tested directory. */
  name: string;
  cases: CaseReport[];
}

export interface TestReport {
  files: number;
  scenarios: number;
  failures: number;
  duration_ms: number;
  passed: boolean;
  suites: SuiteReport[];
  error: string | null;
}

export function parseTestReport(stdout: string): TestReport {
  const start = stdout.indexOf("{");
  if (start < 0) throw new Error("voltrus-plc test printed no JSON report");
  const data = JSON.parse(stdout.slice(start)) as Partial<TestReport>;
  if (!Array.isArray(data.suites)) throw new Error("test report has no `suites` array");
  return {
    files: data.files ?? data.suites.length,
    scenarios: data.scenarios ?? 0,
    failures: data.failures ?? 0,
    duration_ms: data.duration_ms ?? 0,
    passed: data.passed ?? false,
    error: data.error ?? null,
    suites: data.suites.map((s) => ({
      name: String(s.name).replace(/\\/g, "/"),
      cases: (s.cases ?? []).map((c) => ({
        name: String(c.name),
        outcome: c.outcome === "passed" || c.outcome === "failed" ? c.outcome : "error",
        duration_ms: c.duration_ms ?? 0,
        failures: c.failures ?? [],
        error: c.error ?? null,
        assertions: c.assertions ?? [],
      })),
    })),
  };
}

export function summarize(report: TestReport): string {
  const failed = report.suites.reduce(
    (n, s) => n + s.cases.filter((c) => c.outcome !== "passed").length,
    0,
  );
  const passed = report.suites.reduce(
    (n, s) => n + s.cases.filter((c) => c.outcome === "passed").length,
    0,
  );
  return `${passed} passed, ${failed} failed (${report.files} files, ${report.duration_ms} ms)`;
}

/** Message lines for a failing case: setup errors, assertion failures, load errors. */
export function failureMessages(c: CaseReport): string[] {
  const out = [...c.failures];
  if (c.error) out.push(c.error);
  if (out.length === 0 && c.outcome !== "passed") out.push(`scenario ${c.outcome}`);
  return out;
}

export interface ScenarioLocation {
  name: string;
  /** 0-based line of the `- name:` entry. */
  line: number;
}

/**
 * Find scenario names in a *.test.yaml without a YAML parser: scenario
 * entries are the only list items carrying a `name:` key (steps carry
 * set/advance/expect).
 */
export function findScenarios(yamlText: string): ScenarioLocation[] {
  const out: ScenarioLocation[] = [];
  const lines = yamlText.split(/\r?\n/);
  const re = /^\s*-\s+name:\s*(.*?)\s*$/;
  for (let i = 0; i < lines.length; i++) {
    const m = re.exec(lines[i]);
    if (!m) continue;
    const name = unquote(stripComment(m[1]));
    if (name) out.push({ name, line: i });
  }
  return out;
}

function stripComment(v: string): string {
  if (v.startsWith('"') || v.startsWith("'")) return v;
  const hash = v.search(/\s#/);
  return (hash >= 0 ? v.slice(0, hash) : v).trim();
}

function unquote(v: string): string {
  const t = v.trim();
  if (t.length >= 2 && t.startsWith('"')) {
    const end = t.lastIndexOf('"');
    if (end > 0) return t.slice(1, end).replace(/\\"/g, '"');
  }
  if (t.length >= 2 && t.startsWith("'")) {
    const end = t.lastIndexOf("'");
    if (end > 0) return t.slice(1, end).replace(/''/g, "'");
  }
  return t;
}
