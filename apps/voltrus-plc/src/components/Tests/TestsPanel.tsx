import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useStore } from "../../store";
import type { ChatMessage, FileEntry } from "../../types";

// Mirrors the wire format of commands::tests::TestReportDto (snake_case fields).
export interface AssertionDto {
  name: string;
  passed: boolean;
  expected: string;
  actual: string;
}

export interface ScenarioDto {
  name: string;
  outcome: "passed" | "failed" | "error";
  duration_ms: number;
  error: string | null;
  assertions: AssertionDto[];
}

export interface TestSuiteDto {
  path: string;
  scenarios: ScenarioDto[];
}

export interface TestReportDto {
  passed: boolean;
  duration_ms: number;
  file_count: number;
  scenario_count: number;
  failure_count: number;
  error: string | null;
  suites: TestSuiteDto[];
}

const OUTCOME_GLYPHS: Record<ScenarioDto["outcome"], string> = {
  passed: "\u2713",
  failed: "\u2717",
  error: "!",
};

const OUTCOME_COLORS: Record<ScenarioDto["outcome"], string> = {
  passed: "#10b981",
  failed: "var(--danger)",
  error: "#f59e0b",
};

export function collectTestFiles(files: FileEntry[]): FileEntry[] {
  return files.filter((f) => !f.is_dir && f.name.endsWith(".test.yaml"));
}

export function extractYaml(response: string): string {
  const match = response.match(/```(?:yaml|yml)?\s*\n([\s\S]*?)```/i);
  return (match ? match[1] : response).trim();
}

export function targetTestPath(projectDir: string, programName: string): string {
  const base = programName.replace(/\.(st|txt|fbd|ld|sfc)$/i, "");
  return `${projectDir}/${base}.test.yaml`;
}

function scenarioKey(path: string, name: string): string {
  return `${path}::${name}`;
}

export default function TestsPanel() {
  const projectDir = useStore((s) => s.projectDir);
  const openFiles = useStore((s) => s.openFiles);
  const activeFileIndex = useStore((s) => s.activeFileIndex);
  const compiledVariables = useStore((s) => s.compiledVariables);
  const refreshProjectFiles = useStore((s) => s.refreshProjectFiles);
  const aiProvider = useStore((s) => s.aiProvider);
  const aiApiKey = useStore((s) => s.aiApiKey);
  const aiModel = useStore((s) => s.aiModel);
  const ollamaEndpoint = useStore((s) => s.ollamaEndpoint);

  const [testFiles, setTestFiles] = useState<FileEntry[]>([]);
  const [report, setReport] = useState<TestReportDto | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [focusFile, setFocusFile] = useState<string | null>(null);

  // Generate-test review dialog ("Save, then Validate" flow).
  const [reviewOpen, setReviewOpen] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [genBuffer, setGenBuffer] = useState("");
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [validating, setValidating] = useState(false);
  const [validateSummary, setValidateSummary] = useState<string | null>(null);

  const activeProgram = openFiles[activeFileIndex];

  useEffect(() => {
    if (!projectDir) {
      setTestFiles([]);
      return;
    }
    invoke<FileEntry[]>("list_project_files", { dir: projectDir })
      .then((files) => setTestFiles(collectTestFiles(files)))
      .catch((e) => setError(String(e)));
  }, [projectDir]);

  const runTests = async (focus: string | null) => {
    if (!projectDir || running) return;
    setRunning(true);
    setError(null);
    try {
      const result = await invoke<TestReportDto>("run_project_tests", {
        projectDir,
        junit: null,
      });
      setReport(result);
      setFocusFile(focus);
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  const generateTest = async () => {
    if (!activeProgram || generating) return;
    setReviewOpen(true);
    setGenerating(true);
    setGenBuffer("");
    setSavedPath(null);
    setValidateSummary(null);

    let full = "";
    let unlistenChunk: UnlistenFn | null = null;
    let unlistenDone: UnlistenFn | null = null;
    let cleanedUp = false;
    const cleanup = () => {
      if (cleanedUp) return;
      cleanedUp = true;
      unlistenChunk?.();
      unlistenDone?.();
      setGenerating(false);
    };

    try {
      unlistenChunk = await listen<string>("ai-chunk", (event) => {
        full += event.payload;
        setGenBuffer(full);
      });
      unlistenDone = await listen("ai-done", () => cleanup());

      const prompt = await invoke<string>("generate_test_prompt", {
        programSource: activeProgram.content,
        variables: compiledVariables.map((v) => v.name),
      });
      const messages: ChatMessage[] = [{ role: "user", content: prompt }];
      if (aiProvider === "claude") {
        await invoke("chat_claude_stream", {
          messages,
          apiKey: aiApiKey,
          model: aiModel,
        });
      } else {
        await invoke("chat_ollama_stream", {
          messages,
          endpoint: ollamaEndpoint,
          model: aiModel,
        });
      }
    } catch (e) {
      setError(String(e));
    } finally {
      cleanup();
    }
  };

  const saveGenerated = async () => {
    if (!projectDir || !activeProgram || saving) return;
    const path = targetTestPath(projectDir, activeProgram.name);
    setSaving(true);
    try {
      await invoke("save_file", { path, content: extractYaml(genBuffer) });
      setSavedPath(path);
      refreshProjectFiles();
      invoke<FileEntry[]>("list_project_files", { dir: projectDir })
        .then((files) => setTestFiles(collectTestFiles(files)))
        .catch(() => {});
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const validateGenerated = async () => {
    if (!savedPath || !projectDir || validating) return;
    setValidating(true);
    setValidateSummary(null);
    try {
      const result = await invoke<TestReportDto>("run_project_tests", {
        projectDir,
        junit: null,
      });
      setReport(result);
      setFocusFile(null);
      const file = result.suites.find(
        (f) => savedPath.endsWith(f.path) || f.path.endsWith(savedPath),
      );
      const total = file ? file.scenarios.length : 0;
      const failed = file
        ? file.scenarios.filter((s) => s.outcome !== "passed").length
        : 0;
      setValidateSummary(`${total - failed}/${total} scenarios passed`);
    } catch (e) {
      setError(String(e));
    } finally {
      setValidating(false);
    }
  };

  const toggleScenario = (key: string) => {
    setExpanded((prev) => ({ ...prev, [key]: !prev[key] }));
  };

  const displayYaml = generating ? genBuffer : extractYaml(genBuffer);
  const files = report
    ? report.suites.filter((f) => !focusFile || f.path === focusFile)
    : [];
  const failedScenarios = report
    ? report.suites.reduce(
        (n, f) => n + f.scenarios.filter((s) => s.outcome !== "passed").length,
        0,
      )
    : 0;
  const totalScenarios = report
    ? report.suites.reduce((n, f) => n + f.scenarios.length, 0)
    : 0;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
        <button
          className="toolbar-btn primary"
          onClick={() => runTests(null)}
          disabled={!projectDir || running}
        >
          {running ? "Running..." : "Run All"}
        </button>
        <button
          className="toolbar-btn"
          onClick={generateTest}
          disabled={!activeProgram || generating}
          title="AI: generate a *.test.yaml scenario file for the open program"
        >
          Generate test
        </button>
        {focusFile && (
          <button
            className="toolbar-btn"
            style={{ color: "var(--accent)", fontSize: 10 }}
            onClick={() => setFocusFile(null)}
          >
            Show all files
          </button>
        )}
        {report && (
          <span
            style={{
              marginLeft: "auto",
              color: report.passed ? "#10b981" : "var(--danger)",
              alignSelf: "center",
            }}
          >
            {totalScenarios - failedScenarios}/{totalScenarios} scenarios passed
            {" \u00b7 "}
            {report.duration_ms} ms
          </span>
        )}
      </div>

      {error && (
        <div style={{ color: "var(--danger)", marginBottom: 8 }}>{error}</div>
      )}
      {report?.error && (
        <div style={{ color: "var(--danger)", marginBottom: 8 }}>
          {report.error}
        </div>
      )}

      {!projectDir && (
        <span style={{ color: "var(--text-muted)" }}>
          Open a folder to run tests
        </span>
      )}

      {projectDir && testFiles.length === 0 && (
        <div style={{ color: "var(--text-muted)" }}>
          No *.test.yaml files in this project. A test file pairs a program with
          virtual-time scenarios (<code>program:</code>, <code>given:</code>,{" "}
          <code>steps:</code> with <code>advance:</code>/<code>expect:</code>)
          and runs deterministically — see{" "}
          <code>voltrus-plc test --help</code> or ask AI to generate one from
          the open program.
        </div>
      )}
      {projectDir && testFiles.length > 0 && !report && (
        <div style={{ color: "var(--text-muted)" }}>
          {testFiles.length} test file{testFiles.length === 1 ? "" : "s"} found
          — Run All or run a single file to see results
          {testFiles.map((f) => (
            <div key={f.path} className="location">
              {f.name}
            </div>
          ))}
        </div>
      )}
      {files.map((f) => (
        <div key={f.path} style={{ marginBottom: 10 }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              fontWeight: 600,
              marginTop: 4,
            }}
          >
            <span className="location">{f.path}</span>
            <button
              className="toolbar-btn"
              style={{ fontSize: 10, padding: "1px 6px", color: "var(--accent)" }}
              onClick={() => runTests(f.path)}
              disabled={running}
            >
              Run
            </button>
          </div>
          {f.scenarios.length === 0 && (
            <div style={{ color: "var(--text-muted)", paddingLeft: 24 }}>
              No scenarios
            </div>
          )}
          {f.scenarios.map((s) => {
            const key = scenarioKey(f.path, s.name);
            const open = !!expanded[key];
            const failedChecks = s.assertions.filter((a) => !a.passed).length;
            return (
              <div key={key}>
                <div
                  className="diagnostic-item"
                  role="button"
                  aria-expanded={open}
                  onClick={() => toggleScenario(key)}
                >
                  <span
                    className={`severity ${s.outcome === "failed" ? "error" : s.outcome === "error" ? "warning" : ""}`}
                    style={{ color: OUTCOME_COLORS[s.outcome] }}
                  >
                    {OUTCOME_GLYPHS[s.outcome]}
                  </span>
                  <span style={{ flex: 1 }}>{s.name}</span>
                  <span style={{ color: "var(--text-muted)" }}>
                    {failedChecks > 0
                      ? `${failedChecks}/${s.assertions.length} failed`
                      : `${s.assertions.length} checks`}
                  </span>
                  <span className="location">{s.duration_ms} ms</span>
                </div>
                {s.error && (
                  <div
                    style={{
                      display: "flex",
                      gap: 8,
                      paddingLeft: 24,
                      color: "#f59e0b",
                    }}
                  >
                    <span>{s.error}</span>
                  </div>
                )}
                {open &&
                  s.assertions.map((a) => (
                    <div
                      key={a.name}
                      style={{
                        display: "flex",
                        gap: 8,
                        paddingLeft: 24,
                        color: a.passed ? "var(--text-muted)" : "var(--danger)",
                      }}
                    >
                      <span
                        style={{
                          width: 16,
                          textAlign: "center",
                          color: a.passed ? "#10b981" : "var(--danger)",
                        }}
                      >
                        {a.passed ? "\u2713" : "\u2717"}
                      </span>
                      <span>{a.name}</span>
                      <span>
                        expected <strong>{a.expected}</strong>
                        {!a.passed && (
                          <>
                            {" , "}got <strong>{a.actual}</strong>
                          </>
                        )}
                      </span>
                    </div>
                  ))}
              </div>
            );
          })}
        </div>
      ))}

      {reviewOpen && (
        <div className="modal-overlay" onClick={() => setReviewOpen(false)}>
          <div
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="test-review-title"
            onClick={(e) => e.stopPropagation()}
            style={{ minWidth: 480, maxWidth: 720 }}
          >
            <h3 id="test-review-title">
              Generated test for {activeProgram?.name ?? "program"}
            </h3>
            <pre
              style={{
                margin: 0,
                maxHeight: 300,
                overflow: "auto",
                fontSize: 11,
                whiteSpace: "pre-wrap",
                color: "var(--text)",
              }}
            >
              {displayYaml || (generating ? "..." : "(empty response)")}
            </pre>
            {validateSummary && (
              <div
                style={{
                  marginTop: 8,
                  color: validateSummary.startsWith("0/")
                    ? "var(--danger)"
                    : "#10b981",
                }}
              >
                {validateSummary}
              </div>
            )}
            <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
              <button
                className="toolbar-btn"
                onClick={saveGenerated}
                disabled={generating || saving}
              >
                {saving ? "Saving..." : "Save"}
              </button>
              <button
                className="toolbar-btn primary"
                onClick={validateGenerated}
                disabled={!savedPath || validating}
              >
                {validating ? "Running..." : "Validate"}
              </button>
              <button
                className="toolbar-btn"
                onClick={() => setReviewOpen(false)}
              >
                Close
              </button>
            </div>
            <div
              style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 8 }}
            >
              {savedPath
                ? `Saved to ${savedPath}`
                : "Save, then Validate — the file must be saved into the project before it can run."}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
