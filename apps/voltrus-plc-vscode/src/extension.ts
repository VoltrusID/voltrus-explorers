import { execFile } from "node:child_process";
import * as path from "node:path";
import * as vscode from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";
import { parseCheckOutput } from "./checkOutput.ts";
import { installToolchain } from "./download.ts";
import { createTesting, type Testing } from "./testing.ts";
import { nodeEnv, resolveBinary, type ToolName } from "./toolchain.ts";

const SETTING: Record<ToolName, string> = {
  "voltrus-lsp": "lspPath",
  "voltrus-plc": "cliPath",
};

let client: LanguageClient | undefined;
let output: vscode.OutputChannel;
let context: vscode.ExtensionContext;
let downloadPrompted = false;
let installing: Promise<boolean> | undefined;

export interface CliResult {
  stdout: string;
  stderr: string;
  code: number;
}

export async function activate(ctx: vscode.ExtensionContext) {
  context = ctx;
  output = vscode.window.createOutputChannel("Voltrus PLC");
  const checkDiagnostics = vscode.languages.createDiagnosticCollection("voltrus-plc");
  const testing: Testing = createTesting(ctx, output, runCli);

  ctx.subscriptions.push(
    output,
    checkDiagnostics,
    vscode.commands.registerCommand("voltrusPlc.checkWorkspace", () =>
      checkWorkspace(checkDiagnostics),
    ),
    vscode.commands.registerCommand("voltrusPlc.runTests", () => testing.runAll()),
    vscode.commands.registerCommand("voltrusPlc.installToolchain", async () => {
      if (await downloadToolchain()) await restartClient();
    }),
    vscode.commands.registerCommand("voltrusPlc.restartLanguageServer", restartClient),
    vscode.commands.registerCommand("voltrusPlc.showOutput", () => output.show()),
    // Check results go stale as soon as a file is edited; the language
    // server takes over diagnostics for open documents.
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (e.contentChanges.length > 0) checkDiagnostics.delete(e.document.uri);
    }),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("voltrusPlc.lspPath")) void restartClient();
    }),
    vscode.tasks.registerTaskProvider("voltrus-plc", new PlcTaskProvider()),
  );

  // Not awaited: when the binary is missing, startClient waits on a
  // notification, which must not hold up activation.
  void startClient();
  return { getClient: () => client };
}

export async function deactivate() {
  await client?.stop();
}

function workspaceDir(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

/** Resolve a toolchain binary; when absent and `interactive`, offer the download once. */
async function resolveTool(tool: ToolName, interactive: boolean): Promise<string | undefined> {
  const key = SETTING[tool];
  const setting = vscode.workspace.getConfiguration("voltrusPlc").get<string>(key);
  const storage = context.globalStorageUri.fsPath;
  let res = resolveBinary(tool, setting, storage, nodeEnv(), workspaceDir());
  if (res.kind === "found") return res.path;
  if (res.kind === "missing-setting") {
    void vscode.window
      .showErrorMessage(`voltrusPlc.${key} points to ${res.path}, which does not exist.`, "Open Settings")
      .then((pick) => {
        if (pick) void vscode.commands.executeCommand("workbench.action.openSettings", `voltrusPlc.${key}`);
      });
    return undefined;
  }
  if (!interactive) return undefined;
  const pick = await vscode.window.showInformationMessage(
    `Voltrus PLC needs ${tool}. Download the free toolchain (voltrus-lsp + voltrus-plc), or set its path?`,
    "Download",
    "Set Path",
  );
  if (pick === "Set Path") {
    void vscode.commands.executeCommand("workbench.action.openSettings", `voltrusPlc.${key}`);
    return undefined;
  }
  if (pick !== "Download" || !(await downloadToolchain())) return undefined;
  res = resolveBinary(tool, setting, storage, nodeEnv(), workspaceDir());
  return res.kind === "found" ? res.path : undefined;
}

async function downloadToolchain(): Promise<boolean> {
  installing ??= (async () => {
    const url = vscode.workspace
      .getConfiguration("voltrusPlc")
      .get<string>("toolchainManifestUrl", "https://dl-voltrus-plc.voltrus.id/releases/toolchain/latest.json");
    try {
      const result = await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "Voltrus PLC toolchain", cancellable: true },
        (progress, token) => {
          const abort = new AbortController();
          token.onCancellationRequested(() => abort.abort());
          return installToolchain(
            context.globalStorageUri.fsPath,
            url,
            (message) => progress.report({ message }),
            abort.signal,
          );
        },
      );
      output.appendLine(`Installed toolchain ${result.version} in ${result.dir}`);
      void vscode.window.showInformationMessage(`Voltrus PLC toolchain ${result.version} installed.`);
      return true;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      output.appendLine(`Toolchain download failed: ${msg}`);
      void vscode.window.showErrorMessage(`Voltrus PLC toolchain download failed: ${msg}`);
      return false;
    } finally {
      installing = undefined;
    }
  })();
  return installing;
}

async function startClient(): Promise<void> {
  const interactive = !downloadPrompted;
  downloadPrompted = true;
  const lsp = await resolveTool("voltrus-lsp", interactive);
  if (!lsp) {
    output.appendLine("voltrus-lsp not found: language features are off. Run 'Voltrus PLC: Download Toolchain' or set voltrusPlc.lspPath.");
    return;
  }
  output.appendLine(`Starting ${lsp}`);
  const serverOptions: ServerOptions = {
    run: { command: lsp, transport: TransportKind.stdio },
    debug: { command: lsp, transport: TransportKind.stdio },
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "iecst" },
      { scheme: "untitled", language: "iecst" },
    ],
    outputChannel: output,
  };
  client = new LanguageClient("voltrusPlc", "Voltrus PLC Language Server", serverOptions, clientOptions);
  try {
    await client.start();
  } catch (err) {
    output.appendLine(`Language server failed to start: ${err instanceof Error ? err.message : String(err)}`);
  }
}

async function restartClient(): Promise<void> {
  const old = client;
  client = undefined;
  await old?.stop().catch(() => undefined);
  await startClient();
}

/** Run the CLI. Non-zero exits are normal (check exits with the error count). */
export async function runCli(args: string[], cwd: string): Promise<CliResult | undefined> {
  const cli = await resolveTool("voltrus-plc", true);
  if (!cli) return undefined;
  output.appendLine(`> ${cli} ${args.join(" ")}  (in ${cwd})`);
  return new Promise((resolve) => {
    execFile(cli, args, { cwd, maxBuffer: 64 * 1024 * 1024 }, (err, stdout, stderr) => {
      const code = err && typeof (err as { code?: unknown }).code === "number" ? (err as { code: number }).code : err ? -1 : 0;
      if (err && code === -1) output.appendLine(`Failed to run ${cli}: ${err.message}`);
      resolve({ stdout, stderr, code });
    });
  });
}

async function checkWorkspace(collection: vscode.DiagnosticCollection): Promise<void> {
  const folders = vscode.workspace.workspaceFolders ?? [];
  if (folders.length === 0) {
    void vscode.window.showWarningMessage("Open a folder with .st files first.");
    return;
  }
  collection.clear();
  output.show(true);
  let errors = 0;
  let warnings = 0;
  for (const folder of folders) {
    const dir = folder.uri.fsPath;
    const res = await runCli(["check", dir], dir);
    if (!res) return;
    output.append(res.stdout);
    if (res.stderr) output.append(res.stderr);
    const parsed = parseCheckOutput(res.stdout);
    const byFile = new Map<string, vscode.Diagnostic[]>();
    for (const d of parsed.diagnostics) {
      const pos = new vscode.Position(Math.max(0, d.line - 1), Math.max(0, d.column - 1));
      const diag = new vscode.Diagnostic(
        new vscode.Range(pos, pos.translate(0, 1)),
        d.message,
        d.severity === "error" ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning,
      );
      diag.source = "voltrus-plc";
      const file = path.join(dir, d.file);
      byFile.set(file, [...(byFile.get(file) ?? []), diag]);
    }
    for (const [file, diags] of byFile) collection.set(vscode.Uri.file(file), diags);
    errors += parsed.summary?.errors ?? parsed.diagnostics.filter((d) => d.severity === "error").length;
    warnings += parsed.summary?.warnings ?? parsed.diagnostics.filter((d) => d.severity === "warning").length;
  }
  const text = `Voltrus PLC check: ${errors} errors, ${warnings} warnings.`;
  if (errors > 0) {
    void vscode.window.showErrorMessage(text, "Show Problems").then((pick) => {
      if (pick) void vscode.commands.executeCommand("workbench.actions.view.problems");
    });
  } else {
    void vscode.window.showInformationMessage(text);
  }
}

interface PlcTaskDefinition extends vscode.TaskDefinition {
  command: "check" | "test";
  path?: string;
  junit?: string;
}

class PlcTaskProvider implements vscode.TaskProvider {
  async provideTasks(): Promise<vscode.Task[]> {
    const tasks: vscode.Task[] = [];
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      for (const command of ["check", "test"] as const) {
        const task = await this.build({ type: "voltrus-plc", command }, folder);
        if (task) tasks.push(task);
      }
    }
    return tasks;
  }

  async resolveTask(task: vscode.Task): Promise<vscode.Task | undefined> {
    const def = task.definition as PlcTaskDefinition;
    if (def.command !== "check" && def.command !== "test") return undefined;
    const scope = task.scope && typeof task.scope === "object" ? task.scope : vscode.workspace.workspaceFolders?.[0];
    if (!scope) return undefined;
    return this.build(def, scope);
  }

  private async build(def: PlcTaskDefinition, folder: vscode.WorkspaceFolder): Promise<vscode.Task> {
    const cli = (await resolveTool("voltrus-plc", false)) ?? "voltrus-plc";
    const target = def.path ? path.resolve(folder.uri.fsPath, def.path) : folder.uri.fsPath;
    const args = [def.command, target];
    if (def.command === "test" && def.junit) args.push("--junit", def.junit);
    const task = new vscode.Task(
      def,
      folder,
      def.command,
      "voltrus-plc",
      new vscode.ProcessExecution(cli, args, { cwd: folder.uri.fsPath }),
      def.command === "check" ? ["$voltrus-plc"] : [],
    );
    task.group = def.command === "check" ? vscode.TaskGroup.Build : vscode.TaskGroup.Test;
    return task;
  }
}
