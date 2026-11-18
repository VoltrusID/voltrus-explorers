// Parser for `voltrus-plc check` output:
//   Broken.st:5:7: error: error: undefined identifier 'y' at 45..46
// File paths are relative to the checked directory. The compiler repeats
// the severity inside the message and appends a byte span; both are
// dropped. Keep CHECK_LINE_SOURCE identical to the `$voltrus-plc`
// problemMatcher regexp in package.json (a unit test enforces it).

export const CHECK_LINE_SOURCE =
  "^(.+?):(\\d+):(\\d+): (error|warning): (?:(?:error|warning): )?(.*?)(?: at \\d+(?:\\.\\.\\d+)?)?$";

export interface CheckDiagnostic {
  /** Path relative to the checked directory, forward slashes. */
  file: string;
  /** 1-based. */
  line: number;
  /** 1-based. */
  column: number;
  severity: "error" | "warning";
  message: string;
}

export interface CheckSummary {
  files: number;
  errors: number;
  warnings: number;
}

const SUMMARY_RE = /^checked (\d+) files?, (\d+) errors?, (\d+) warnings?$/;

export function parseCheckOutput(text: string): {
  diagnostics: CheckDiagnostic[];
  summary?: CheckSummary;
} {
  const lineRe = new RegExp(CHECK_LINE_SOURCE);
  const diagnostics: CheckDiagnostic[] = [];
  let summary: CheckSummary | undefined;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trimEnd();
    const s = SUMMARY_RE.exec(line);
    if (s) {
      summary = { files: Number(s[1]), errors: Number(s[2]), warnings: Number(s[3]) };
      continue;
    }
    const m = lineRe.exec(line);
    if (!m) continue;
    diagnostics.push({
      file: m[1].replace(/\\/g, "/"),
      line: Number(m[2]),
      column: Number(m[3]),
      severity: m[4] as "error" | "warning",
      message: m[5],
    });
  }
  return { diagnostics, summary };
}
