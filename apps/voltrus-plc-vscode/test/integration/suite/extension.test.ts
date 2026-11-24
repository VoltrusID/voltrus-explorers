import assert from "node:assert/strict";
import * as vscode from "vscode";

const folder = () => vscode.workspace.workspaceFolders![0].uri;

async function waitFor<T>(what: string, fn: () => T | undefined, ms = 30_000): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = fn();
    if (v !== undefined) return v;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`timed out waiting for ${what}`);
}

describe("Voltrus PLC extension", () => {
  before(async () => {
    const ext = vscode.extensions.getExtension("voltrus.voltrus-plc");
    assert.ok(ext, "extension is installed");
    await ext.activate();
  });

  it("registers .st files as iecst", async () => {
    const doc = await vscode.workspace.openTextDocument(vscode.Uri.joinPath(folder(), "TankLevel.st"));
    assert.equal(doc.languageId, "iecst");
  });

  it("language server publishes a diagnostic for a broken file", async () => {
    const uri = vscode.Uri.joinPath(folder(), "Broken.st");
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    const diag = await waitFor("LSP diagnostic", () =>
      vscode.languages
        .getDiagnostics(uri)
        .find((d) => d.source?.startsWith("mt-stc") && /undefined identifier 'y'/.test(d.message)),
    );
    assert.equal(diag.severity, vscode.DiagnosticSeverity.Error);
    assert.equal(diag.range.start.line, 4);
  });

  it("language server answers hover and completion", async () => {
    const uri = vscode.Uri.joinPath(folder(), "TankLevel.st");
    const doc = await vscode.workspace.openTextDocument(uri);
    const line = doc.getText().split("\n").findIndex((l) => l.includes("cycles := cycles + 1"));
    const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
      "vscode.executeHoverProvider",
      uri,
      new vscode.Position(line, 6),
    );
    const text = hovers
      .flatMap((h) => h.contents)
      .map((c) => (typeof c === "string" ? c : c.value))
      .join("\n");
    assert.match(text, /DINT/);

    const list = await vscode.commands.executeCommand<vscode.CompletionList>(
      "vscode.executeCompletionItemProvider",
      uri,
      new vscode.Position(line, 4),
    );
    const labels = list.items.map((i) => (typeof i.label === "string" ? i.label : i.label.label));
    assert.ok(labels.includes("level_setpoint"), "completion offers declared variables");
  });

  it("Check Workspace reports CLI diagnostics", async () => {
    await vscode.commands.executeCommand("voltrusPlc.checkWorkspace");
    const uri = vscode.Uri.joinPath(folder(), "Broken.st");
    const fromCli = vscode.languages.getDiagnostics(uri).filter((d) => d.source === "voltrus-plc");
    assert.ok(fromCli.some((d) => d.message === "undefined identifier 'y'" && d.range.start.line === 4));
  });

  it("Run Tests runs every scenario and passes", async () => {
    const reports = await vscode.commands.executeCommand<{ passed: boolean; scenarios: number }[]>(
      "voltrusPlc.runTests",
    );
    assert.equal(reports.length, 1);
    assert.equal(reports[0].scenarios, 4);
    assert.equal(reports[0].passed, true);
  });

  it("provides check and test tasks", async () => {
    const tasks = await vscode.tasks.fetchTasks({ type: "voltrus-plc" });
    assert.deepEqual(tasks.map((t) => t.name).sort(), ["check", "test"]);
  });
});
