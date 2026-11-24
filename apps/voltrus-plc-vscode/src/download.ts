// Network + extraction half of the toolchain installer. Kept apart from
// toolchain.ts so the unit-tested logic there never touches the network.

import { execFile } from "node:child_process";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import { promisify } from "node:util";
import {
  archiveKind,
  exeName,
  type Manifest,
  parseManifest,
  type PlatformKey,
  platformKey,
  toolchainRoot,
  verifySha256,
} from "./toolchain.ts";

const run = promisify(execFile);

export interface InstallResult {
  version: string;
  dir: string;
}

async function fetchBytes(url: string, signal?: AbortSignal): Promise<Uint8Array> {
  const res = await fetch(url, { signal, redirect: "follow" });
  if (!res.ok) throw new Error(`GET ${url} failed: HTTP ${res.status}`);
  return new Uint8Array(await res.arrayBuffer());
}

export async function fetchManifest(url: string, signal?: AbortSignal): Promise<Manifest> {
  const bytes = await fetchBytes(url, signal);
  return parseManifest(new TextDecoder().decode(bytes));
}

async function extract(archive: string, kind: "tar.gz" | "zip", dest: string): Promise<void> {
  // System tar: GNU tar on Linux, bsdtar on macOS and Windows 10+ (bsdtar
  // also reads .zip). PowerShell is the fallback for zips on older Windows.
  try {
    await run("tar", [kind === "zip" ? "-xf" : "-xzf", archive, "-C", dest]);
  } catch (err) {
    if (kind !== "zip" || process.platform !== "win32") throw err;
    await run("powershell.exe", [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      `Expand-Archive -LiteralPath '${archive.replace(/'/g, "''")}' -DestinationPath '${dest.replace(/'/g, "''")}' -Force`,
    ]);
  }
}

/**
 * Download the toolchain for this machine, verify its sha256, extract
 * voltrus-lsp and voltrus-plc into `<storage>/toolchain/<version>/` and mark
 * that version current.
 */
export async function installToolchain(
  storageDir: string,
  manifestUrl: string,
  report: (message: string) => void = () => {},
  signal?: AbortSignal,
): Promise<InstallResult> {
  const key: PlatformKey | undefined = platformKey(process.platform, process.arch);
  if (!key) throw new Error(`no prebuilt toolchain for ${process.platform}/${process.arch}`);

  report("Reading release manifest");
  const manifest = await fetchManifest(manifestUrl, signal);
  const entry = manifest.files[key];
  if (!entry) throw new Error(`release ${manifest.version} has no build for ${key}`);
  const kind = archiveKind(entry.url);

  report(`Downloading ${manifest.version} (${key})`);
  const bytes = await fetchBytes(entry.url, signal);
  verifySha256(bytes, entry.sha256);

  const root = toolchainRoot(storageDir);
  await fs.mkdir(root, { recursive: true });
  const staging = await fs.mkdtemp(path.join(root, ".staging-"));
  try {
    report("Extracting");
    const archive = path.join(staging, kind === "zip" ? "toolchain.zip" : "toolchain.tar.gz");
    await fs.writeFile(archive, bytes);
    const unpacked = path.join(staging, "unpacked");
    await fs.mkdir(unpacked);
    await extract(archive, kind, unpacked);

    const finalDir = path.join(root, manifest.version);
    await fs.rm(finalDir, { recursive: true, force: true });
    await fs.mkdir(finalDir, { recursive: true });
    for (const tool of ["voltrus-lsp", "voltrus-plc"] as const) {
      const name = exeName(tool, process.platform);
      const src = path.join(unpacked, name);
      const stat = await fs.stat(src).catch(() => undefined);
      if (!stat?.isFile()) throw new Error(`archive does not contain ${name} at its root`);
      const dst = path.join(finalDir, name);
      await fs.copyFile(src, dst);
      if (process.platform !== "win32") await fs.chmod(dst, 0o755);
    }
    await fs.writeFile(
      path.join(root, "current.json"),
      JSON.stringify({ version: manifest.version }) + "\n",
    );
    return { version: manifest.version, dir: finalDir };
  } finally {
    await fs.rm(staging, { recursive: true, force: true });
  }
}
