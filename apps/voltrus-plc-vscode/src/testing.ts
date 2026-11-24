// Testing API integration: one item per *.test.yaml, one child per scenario.
// The CLI has no per-scenario filter, so every run executes the whole
// folder and reports results for the items that were requested.

import * as vscode from "vscode";
import type { CliResult } from "./extension.ts";
import {
  failureMessages,
  findScenarios,
  parseTestReport,
  summarize,
  type TestReport,
} from "./testReport.ts";

export interface Testing {
  runAll(): Promise<TestReport[] | undefined>;
}

type RunCli = (args: string[], cwd: string) => Promise<CliResult | undefined>;

const EXCLUDE = "**/{node_modules,target,.git}/**";

export function createTesting(
  ctx: vscode.ExtensionContext,
  output: vscode.OutputChannel,
  runCli: RunCli,
): Testing {
  const ctrl = vscode.tests.createTestController("voltrusPlc", "Voltrus PLC");
  ctx.subscriptions.push(ctrl);

  const fileItem = (uri: vscode.Uri): vscode.TestItem => {
    const id = uri.toString();
    let item = ctrl.items.get(id);
    if (!item) {
      item = ctrl.createTestItem(id, vscode.workspace.asRelativePath(uri), uri);
      item.canResolveChildren = true;
      ctrl.items.add(item);
    }
    return item;
  };

  const scenarioItem = (file: vscode.TestItem, name: string): vscode.TestItem => {
    const id = `${file.id}::${name}`;
    let item = file.children.get(id);
    if (!item) {
      item = ctrl.createTestItem(id, name, file.uri);
      file.children.add(item);
    }
    return item;
  };

  const parseFile = async (item: vscode.TestItem) => {
    if (!item.uri) return;
    let text: string;
    try {
      text = new TextDecoder().decode(await vscode.workspace.fs.readFile(item.uri));
    } catch {
      return;
    }
    const children: vscode.TestItem[] = [];
    for (const s of findScenarios(text)) {
      const child = ctrl.createTestItem(`${item.id}::${s.name}`, s.name, item.uri);
      child.range = new vscode.Range(s.line, 0, s.line, 0);
      children.push(child);
    }
    item.children.replace(children);
  };

  const discover = async () => {
    const uris = await vscode.workspace.findFiles("**/*.test.yaml", EXCLUDE);
    await Promise.all(uris.map((u) => parseFile(fileItem(u))));
  };

  ctrl.resolveHandler = async (item) => {
    if (item) await parseFile(item);
    else await discover();
  };

  const watcher = vscode.workspace.createFileSystemWatcher("**/*.test.yaml");
  ctx.subscriptions.push(
    watcher,
    watcher.onDidCreate((u) => void parseFile(fileItem(u))),
    watcher.onDidChange((u) => void parseFile(fileItem(u))),
    watcher.onDidDelete((u) => ctrl.items.delete(u.toString())),
  );

  const execute = async (
    request: vscode.TestRunRequest,
    token?: vscode.CancellationToken,
  ): Promise<TestReport[] | undefined> => {
    await discover();
    const run = ctrl.createTestRun(request, "Voltrus PLC", true);
    const wanted = (item: vscode.TestItem): boolean => {
      if (request.exclude?.some((x) => x === item || x === item.parent)) return false;
      if (!request.include) return true;
      return request.include.some((x) => x === item || x === item.parent);
    };

    const folders = new Set<vscode.WorkspaceFolder>();
    const roots = request.include ?? [...iterate(ctrl.items)];
    for (const item of roots) {
      const folder = item.uri && vscode.workspace.getWorkspaceFolder(item.uri);
      if (folder) folders.add(folder);
    }
    if (!request.include) for (const f of vscode.workspace.workspaceFolders ?? []) folders.add(f);

    for (const file of iterate(ctrl.items)) {
      for (const c of iterate(file.children)) if (wanted(c)) run.enqueued(c);
    }

    const reports: TestReport[] = [];
    try {
      for (const folder of folders) {
        if (token?.isCancellationRequested) break;
        const dir = folder.uri.fsPath;
        const res = await runCli(["test", dir, "--json"], dir);
        if (!res) {
          run.end();
          return undefined;
        }
        let report: TestReport;
        try {
          report = parseTestReport(res.stdout);
        } catch (err) {
          const msg = `${err instanceof Error ? err.message : String(err)}\n${res.stderr}`;
          output.appendLine(msg);
          run.appendOutput(msg.replace(/\n/g, "\r\n"));
          continue;
        }
        reports.push(report);
        output.appendLine(`${folder.name}: ${summarize(report)}`);
        if (report.error) output.appendLine(`  error: ${report.error}`);

        for (const suite of report.suites) {
          const file = fileItem(vscode.Uri.joinPath(folder.uri, ...suite.name.split("/")));
          for (const c of suite.cases) {
            // A file that fails to load or compile is reported as a single
            // pseudo-case named "load"; attach that to the file item.
            const isLoad = c.name === "load" && c.outcome === "error" && suite.cases.length === 1;
            const item = isLoad ? file : scenarioItem(file, c.name);
            if (!wanted(item) && !(isLoad && [...iterate(file.children)].some(wanted))) continue;
            output.appendLine(`  ${c.outcome === "passed" ? "ok  " : "FAIL"}  ${suite.name} › ${c.name}`);
            if (c.outcome === "passed") {
              run.passed(item, c.duration_ms);
              continue;
            }
            const lines = failureMessages(c);
            for (const l of lines) output.appendLine(`        ${l}`);
            const message = new vscode.TestMessage(lines.join("\n"));
            const failed = c.assertions.find((a) => !a.passed);
            if (failed) {
              message.expectedOutput = failed.expected;
              message.actualOutput = failed.actual;
            }
            if (item.uri && item.range) message.location = new vscode.Location(item.uri, item.range);
            if (isLoad) {
              for (const child of iterate(file.children)) if (wanted(child)) run.errored(child, message);
              run.errored(file, message);
            } else if (c.outcome === "error") {
              run.errored(item, message, c.duration_ms);
            } else {
              run.failed(item, message, c.duration_ms);
            }
          }
        }
      }
    } finally {
      run.end();
    }
    return reports;
  };

  ctrl.createRunProfile("Run", vscode.TestRunProfileKind.Run, (req, token) => void execute(req, token), true);

  void discover();

  return {
    async runAll() {
      if (!vscode.workspace.workspaceFolders?.length) {
        void vscode.window.showWarningMessage("Open a folder with *.test.yaml files first.");
        return undefined;
      }
      output.show(true);
      const reports = await execute(new vscode.TestRunRequest());
      if (!reports) return undefined;
      const passed = reports.reduce((n, r) => n + r.suites.flatMap((s) => s.cases).filter((c) => c.outcome === "passed").length, 0);
      const total = reports.reduce((n, r) => n + r.suites.flatMap((s) => s.cases).length, 0);
      const failed = total - passed;
      const text = `Voltrus PLC tests: ${passed} passed, ${failed} failed.`;
      if (failed > 0) {
        void vscode.window.showErrorMessage(text, "Show Test Results").then((pick) => {
          if (pick) void vscode.commands.executeCommand("workbench.view.testing.focus");
        });
      } else {
        void vscode.window.showInformationMessage(text);
      }
      return reports;
    },
  };
}

function* iterate(collection: vscode.TestItemCollection): Generator<vscode.TestItem> {
  for (const [, item] of collection) yield item;
}
