import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { describe, it } from "node:test";
import {
  archiveKind,
  type Env,
  expandPath,
  findOnPath,
  parseManifest,
  platformKey,
  resolveBinary,
  sha256Hex,
  verifySha256,
} from "../../src/toolchain.ts";

const SHA = "a".repeat(64);

function fakeEnv(files: Record<string, string | true>, overrides: Partial<Env> = {}): Env {
  return {
    platform: "linux",
    pathVar: "/usr/local/bin:/usr/bin",
    homeDir: "/home/u",
    isFile: (p) => p in files,
    readFile: (p) => (typeof files[p] === "string" ? (files[p] as string) : undefined),
    ...overrides,
  };
}

describe("platformKey", () => {
  it("maps supported platform/arch pairs", () => {
    assert.equal(platformKey("linux", "x64"), "linux-x86_64");
    assert.equal(platformKey("linux", "arm64"), "linux-aarch64");
    assert.equal(platformKey("darwin", "arm64"), "macos-aarch64");
    assert.equal(platformKey("darwin", "x64"), "macos-x86_64");
    assert.equal(platformKey("win32", "x64"), "windows-x86_64");
    assert.equal(platformKey("win32", "arm64"), "windows-x86_64");
  });
  it("returns undefined for unsupported targets", () => {
    assert.equal(platformKey("linux", "ia32"), undefined);
    assert.equal(platformKey("freebsd", "x64"), undefined);
    assert.equal(platformKey("linux", "arm"), undefined);
  });
});

describe("parseManifest", () => {
  const valid = {
    version: "0.4.2",
    files: {
      "linux-x86_64": { url: "https://dl.example/t-linux.tar.gz", sha256: SHA.toUpperCase() },
      "windows-x86_64": { url: "https://dl.example/t-win.zip", sha256: SHA },
      "plan9-mips": { url: "whatever", sha256: "x" },
    },
  };
  it("accepts a valid manifest from a string, lower-cases hashes, skips unknown platforms", () => {
    const m = parseManifest(JSON.stringify(valid));
    assert.equal(m.version, "0.4.2");
    assert.deepEqual(Object.keys(m.files).sort(), ["linux-x86_64", "windows-x86_64"]);
    assert.equal(m.files["linux-x86_64"]?.sha256, SHA);
  });
  it("rejects bad versions, missing files, non-https urls and bad hashes", () => {
    assert.throws(() => parseManifest("[]"), /version/);
    assert.throws(() => parseManifest({ version: "latest", files: {} }), /invalid version/);
    assert.throws(() => parseManifest({ version: "1.0.0" }), /files/);
    assert.throws(
      () => parseManifest({ version: "1.0.0", files: { "linux-x86_64": { url: "http://x/a.tar.gz", sha256: SHA } } }),
      /https/,
    );
    assert.throws(
      () => parseManifest({ version: "1.0.0", files: { "macos-aarch64": { url: "https://x/a.tar.gz", sha256: "abc" } } }),
      /64 hex/,
    );
    assert.throws(() => parseManifest("{not json"), SyntaxError);
  });
});

describe("sha256", () => {
  const data = new TextEncoder().encode("voltrus");
  const hex = createHash("sha256").update("voltrus").digest("hex");
  it("hashes and verifies case-insensitively", () => {
    assert.equal(sha256Hex(data), hex);
    verifySha256(data, hex.toUpperCase());
  });
  it("throws on mismatch", () => {
    assert.throws(() => verifySha256(data, SHA), /checksum mismatch/);
  });
});

describe("archiveKind", () => {
  it("detects tar.gz and zip, ignoring query strings", () => {
    assert.equal(archiveKind("https://x/a/voltrus-toolchain-linux.tar.gz"), "tar.gz");
    assert.equal(archiveKind("https://x/a.tgz?sig=1"), "tar.gz");
    assert.equal(archiveKind("https://x/a.ZIP"), "zip");
    assert.throws(() => archiveKind("https://x/a.7z"), /unsupported/);
  });
});

describe("findOnPath", () => {
  it("finds the first match on a POSIX PATH", () => {
    const env = fakeEnv({ "/usr/bin/voltrus-lsp": true });
    assert.equal(findOnPath("voltrus-lsp", env), "/usr/bin/voltrus-lsp");
    assert.equal(findOnPath("voltrus-plc", env), undefined);
  });
  it("applies PATHEXT on Windows", () => {
    const env = fakeEnv(
      { "C:\\tools\\voltrus-plc.exe": true },
      { platform: "win32", pathVar: "C:\\Windows;\"C:\\tools\"", pathExt: ".COM;.EXE" },
    );
    assert.equal(findOnPath("voltrus-plc", env), "C:\\tools\\voltrus-plc.exe");
  });
});

describe("resolveBinary", () => {
  const storage = "/home/u/.config/Code/globalStorage/voltrus.voltrus-plc";
  const current = `${storage}/toolchain/current.json`;
  const downloaded = `${storage}/toolchain/0.4.2/voltrus-lsp`;

  it("prefers the setting, expanding ~ and workspace-relative paths", () => {
    const env = fakeEnv({ "/home/u/bin/voltrus-lsp": true, "/usr/bin/voltrus-lsp": true });
    assert.deepEqual(resolveBinary("voltrus-lsp", "~/bin/voltrus-lsp", storage, env), {
      kind: "found",
      path: "/home/u/bin/voltrus-lsp",
      source: "setting",
    });
    const env2 = fakeEnv({ "/ws/tools/voltrus-lsp": true });
    assert.equal(
      (resolveBinary("voltrus-lsp", "tools/voltrus-lsp", storage, env2, "/ws") as { path: string }).path,
      "/ws/tools/voltrus-lsp",
    );
  });
  it("reports a setting that points nowhere instead of falling through", () => {
    const env = fakeEnv({ "/usr/bin/voltrus-lsp": true });
    assert.deepEqual(resolveBinary("voltrus-lsp", "/nope/voltrus-lsp", storage, env), {
      kind: "missing-setting",
      path: "/nope/voltrus-lsp",
    });
  });
  it("uses PATH before the downloaded copy", () => {
    const env = fakeEnv({ "/usr/bin/voltrus-lsp": true, [current]: '{"version":"0.4.2"}', [downloaded]: true });
    assert.equal((resolveBinary("voltrus-lsp", "", storage, env) as { source: string }).source, "path");
  });
  it("falls back to the downloaded copy named in current.json", () => {
    const env = fakeEnv({ [current]: '{"version":"0.4.2"}', [downloaded]: true });
    assert.deepEqual(resolveBinary("voltrus-lsp", undefined, storage, env), {
      kind: "found",
      path: downloaded,
      source: "download",
    });
  });
  it("is not-found when nothing exists or current.json is junk", () => {
    assert.deepEqual(resolveBinary("voltrus-plc", "", storage, fakeEnv({})), { kind: "not-found" });
    const env = fakeEnv({ [current]: "{oops", [downloaded]: true });
    assert.deepEqual(resolveBinary("voltrus-lsp", "", storage, env), { kind: "not-found" });
  });
});

describe("expandPath", () => {
  it("substitutes ${workspaceFolder}", () => {
    assert.equal(expandPath("${workspaceFolder}/bin/x", "/h", "/ws"), "/ws/bin/x");
    assert.equal(expandPath("/abs/x", "/h", "/ws"), "/abs/x");
  });
});
