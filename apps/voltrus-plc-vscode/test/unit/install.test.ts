// installToolchain end to end with a fake fetch and a real tar.gz built
// with the system tar: no network.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { after, describe, it } from "node:test";
import { installToolchain } from "../../src/download.ts";
import { installedBinary, nodeEnv, platformKey, sha256Hex } from "../../src/toolchain.ts";

const key = platformKey(process.platform, process.arch);
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "voltrus-vsc-"));
after(() => fs.rmSync(tmp, { recursive: true, force: true }));

function buildArchive(names: string[]): Uint8Array {
  const src = fs.mkdtempSync(path.join(tmp, "src-"));
  for (const n of names) fs.writeFileSync(path.join(src, n), "#!/bin/sh\necho ok\n");
  const out = path.join(tmp, `t-${Date.now()}-${Math.random()}.tar.gz`);
  execFileSync("tar", ["-czf", out, "-C", src, ...names]);
  return fs.readFileSync(out);
}

function fakeFetch(routes: Record<string, Uint8Array | string>) {
  globalThis.fetch = (async (input: string | URL) => {
    const body = routes[String(input)];
    if (body === undefined) return new Response("missing", { status: 404 });
    return new Response(body);
  }) as typeof fetch;
}

describe("installToolchain", { skip: process.platform === "win32" || !key }, () => {
  const manifestUrl = "https://dl.example/latest.json";
  const archiveUrl = "https://dl.example/toolchain.tar.gz";
  const realFetch = globalThis.fetch;
  after(() => {
    globalThis.fetch = realFetch;
  });

  it("downloads, verifies, extracts and marks the version current", async () => {
    const archive = buildArchive(["voltrus-lsp", "voltrus-plc", "README.txt"]);
    fakeFetch({
      [manifestUrl]: JSON.stringify({ version: "1.2.3", files: { [key!]: { url: archiveUrl, sha256: sha256Hex(archive) } } }),
      [archiveUrl]: archive,
    });
    const storage = path.join(tmp, "storage-ok");
    const res = await installToolchain(storage, manifestUrl);
    assert.equal(res.version, "1.2.3");
    const lsp = installedBinary(storage, "voltrus-lsp", nodeEnv());
    assert.equal(lsp, path.join(storage, "toolchain", "1.2.3", "voltrus-lsp"));
    assert.ok(fs.statSync(lsp!).mode & 0o111, "binary is executable");
    assert.ok(installedBinary(storage, "voltrus-plc", nodeEnv()));
    assert.deepEqual(
      fs.readdirSync(path.join(storage, "toolchain")).sort(),
      ["1.2.3", "current.json"],
      "staging dir is cleaned up",
    );
  });

  it("refuses an archive whose checksum does not match", async () => {
    const archive = buildArchive(["voltrus-lsp", "voltrus-plc"]);
    fakeFetch({
      [manifestUrl]: JSON.stringify({ version: "1.2.3", files: { [key!]: { url: archiveUrl, sha256: "0".repeat(64) } } }),
      [archiveUrl]: archive,
    });
    const storage = path.join(tmp, "storage-bad");
    await assert.rejects(installToolchain(storage, manifestUrl), /checksum mismatch/);
    assert.equal(installedBinary(storage, "voltrus-lsp", nodeEnv()), undefined);
  });

  it("fails when a binary is missing from the archive", async () => {
    const archive = buildArchive(["voltrus-lsp"]);
    fakeFetch({
      [manifestUrl]: JSON.stringify({ version: "1.2.3", files: { [key!]: { url: archiveUrl, sha256: sha256Hex(archive) } } }),
      [archiveUrl]: archive,
    });
    await assert.rejects(installToolchain(path.join(tmp, "storage-missing"), manifestUrl), /voltrus-plc/);
  });

  it("fails when the manifest has no build for this platform", async () => {
    fakeFetch({ [manifestUrl]: JSON.stringify({ version: "1.2.3", files: {} }) });
    await assert.rejects(installToolchain(path.join(tmp, "storage-none"), manifestUrl), /no build for/);
  });
});
