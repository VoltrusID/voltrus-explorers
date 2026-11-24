// Launches a downloaded VS Code with this extension against a temp copy of
// test/fixtures/workspace. Needs real binaries:
//   VOLTRUS_LSP=/path/to/voltrus-lsp VOLTRUS_PLC=/path/to/voltrus-plc pnpm test:integration
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { runTests } from "@vscode/test-electron";

async function main() {
  const root = path.resolve(__dirname, "..");
  const lsp = process.env.VOLTRUS_LSP;
  const cli = process.env.VOLTRUS_PLC;
  if (!lsp || !cli) throw new Error("set VOLTRUS_LSP and VOLTRUS_PLC to the toolchain binaries");

  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "voltrus-plc-ws-"));
  fs.cpSync(path.join(root, "test", "fixtures", "workspace"), workspace, { recursive: true });
  fs.mkdirSync(path.join(workspace, ".vscode"));
  fs.writeFileSync(
    path.join(workspace, ".vscode", "settings.json"),
    JSON.stringify({ "voltrusPlc.lspPath": lsp, "voltrusPlc.cliPath": cli }, null, 2),
  );

  try {
    await runTests({
      version: process.env.VSCODE_VERSION ?? "stable",
      extensionDevelopmentPath: root,
      extensionTestsPath: path.join(root, "dist-test", "suite", "index.js"),
      launchArgs: [...(process.env.VSCODE_EXTRA_ARGS?.split(" ").filter(Boolean) ?? []), workspace, "--disable-extensions", "--disable-workspace-trust", "--skip-welcome", "--skip-release-notes"],
    });
  } finally {
    fs.rmSync(workspace, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
