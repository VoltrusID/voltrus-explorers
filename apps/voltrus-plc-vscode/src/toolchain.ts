// Locating the voltrus-lsp / voltrus-plc binaries and validating toolchain
// downloads. No `vscode` import: everything here is unit-tested under plain
// Node, with file-system access injected.

import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

export type ToolName = "voltrus-lsp" | "voltrus-plc";

export const PLATFORMS = [
  "linux-x86_64",
  "linux-aarch64",
  "macos-aarch64",
  "macos-x86_64",
  "windows-x86_64",
] as const;
export type PlatformKey = (typeof PLATFORMS)[number];

/** Map Node's `process.platform` / `process.arch` to a manifest platform key. */
export function platformKey(platform: string, arch: string): PlatformKey | undefined {
  if (platform === "linux" && arch === "x64") return "linux-x86_64";
  if (platform === "linux" && arch === "arm64") return "linux-aarch64";
  if (platform === "darwin" && arch === "arm64") return "macos-aarch64";
  if (platform === "darwin" && arch === "x64") return "macos-x86_64";
  // Windows on ARM runs x64 binaries through emulation.
  if (platform === "win32" && (arch === "x64" || arch === "arm64")) return "windows-x86_64";
  return undefined;
}

export function exeName(tool: ToolName, platform: string): string {
  return platform === "win32" ? `${tool}.exe` : tool;
}

export interface ManifestFile {
  url: string;
  sha256: string;
}

export interface Manifest {
  version: string;
  files: Partial<Record<PlatformKey, ManifestFile>>;
}

const SHA256_RE = /^[0-9a-f]{64}$/i;
const VERSION_RE = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;

/** Validate the `latest.json` manifest. Throws with a readable message. */
export function parseManifest(raw: unknown): Manifest {
  const data = typeof raw === "string" ? (JSON.parse(raw) as unknown) : raw;
  if (!data || typeof data !== "object") throw new Error("manifest is not a JSON object");
  const { version, files } = data as { version?: unknown; files?: unknown };
  if (typeof version !== "string" || !VERSION_RE.test(version)) {
    throw new Error(`manifest has an invalid version: ${JSON.stringify(version)}`);
  }
  if (!files || typeof files !== "object" || Array.isArray(files)) {
    throw new Error("manifest has no `files` object");
  }
  const out: Manifest = { version, files: {} };
  for (const [key, entry] of Object.entries(files as Record<string, unknown>)) {
    // Unknown platforms are skipped so the manifest can grow without breaking
    // older extension versions.
    if (!(PLATFORMS as readonly string[]).includes(key)) continue;
    const { url, sha256 } = (entry ?? {}) as { url?: unknown; sha256?: unknown };
    if (typeof url !== "string" || !url.startsWith("https://")) {
      throw new Error(`manifest entry ${key}: url must be an https URL`);
    }
    if (typeof sha256 !== "string" || !SHA256_RE.test(sha256)) {
      throw new Error(`manifest entry ${key}: sha256 must be 64 hex characters`);
    }
    out.files[key as PlatformKey] = { url, sha256: sha256.toLowerCase() };
  }
  return out;
}

export function sha256Hex(data: Uint8Array): string {
  return createHash("sha256").update(data).digest("hex");
}

/** Throws when `data` does not hash to `expected` (hex, any case). */
export function verifySha256(data: Uint8Array, expected: string): void {
  const actual = sha256Hex(data);
  if (actual !== expected.toLowerCase()) {
    throw new Error(`checksum mismatch: expected ${expected.toLowerCase()}, got ${actual}`);
  }
}

export function archiveKind(url: string): "tar.gz" | "zip" {
  const pathname = new URL(url).pathname.toLowerCase();
  if (pathname.endsWith(".zip")) return "zip";
  if (pathname.endsWith(".tar.gz") || pathname.endsWith(".tgz")) return "tar.gz";
  throw new Error(`unsupported archive type: ${url}`);
}

export interface Env {
  platform: string;
  /** Contents of the PATH environment variable. */
  pathVar: string;
  /** Windows PATHEXT; ignored elsewhere. */
  pathExt?: string;
  homeDir: string;
  isFile: (p: string) => boolean;
  readFile: (p: string) => string | undefined;
}

export function nodeEnv(): Env {
  return {
    platform: process.platform,
    pathVar: process.env.PATH ?? process.env.Path ?? "",
    pathExt: process.env.PATHEXT,
    homeDir: os.homedir(),
    isFile: (p) => {
      try {
        return fs.statSync(p).isFile();
      } catch {
        return false;
      }
    },
    readFile: (p) => {
      try {
        return fs.readFileSync(p, "utf8");
      } catch {
        return undefined;
      }
    },
  };
}

/** Look `name` up on PATH, honouring PATHEXT on Windows. */
export function findOnPath(name: string, env: Env): string | undefined {
  const isWin = env.platform === "win32";
  const sep = isWin ? ";" : ":";
  const join = isWin ? path.win32.join : path.posix.join;
  const exts = isWin
    ? ["", ...(env.pathExt ?? ".EXE;.CMD;.BAT").split(";").filter(Boolean).map((e) => e.toLowerCase())]
    : [""];
  for (const dir of env.pathVar.split(sep)) {
    if (!dir) continue;
    for (const ext of exts) {
      const candidate = join(dir.replace(/^"|"$/g, ""), name + ext);
      if (env.isFile(candidate)) return candidate;
    }
  }
  return undefined;
}

/** `<storage>/toolchain` holds one folder per version plus `current.json`. */
export function toolchainRoot(storageDir: string): string {
  return path.join(storageDir, "toolchain");
}

export function installedVersion(storageDir: string, env: Env): string | undefined {
  const raw = env.readFile(path.join(toolchainRoot(storageDir), "current.json"));
  if (!raw) return undefined;
  try {
    const version = (JSON.parse(raw) as { version?: unknown }).version;
    return typeof version === "string" && VERSION_RE.test(version) ? version : undefined;
  } catch {
    return undefined;
  }
}

export function installedBinary(storageDir: string, tool: ToolName, env: Env): string | undefined {
  const version = installedVersion(storageDir, env);
  if (!version) return undefined;
  const candidate = path.join(toolchainRoot(storageDir), version, exeName(tool, env.platform));
  return env.isFile(candidate) ? candidate : undefined;
}

export type BinarySource = "setting" | "path" | "download";

export type Resolution =
  | { kind: "found"; path: string; source: BinarySource }
  | { kind: "missing-setting"; path: string }
  | { kind: "not-found" };

/**
 * Resolution order: explicit setting → PATH → copy downloaded into global
 * storage. A setting that points at a missing file is reported as such
 * rather than silently falling through, so typos are visible.
 */
export function resolveBinary(
  tool: ToolName,
  setting: string | undefined,
  storageDir: string,
  env: Env,
  workspaceDir?: string,
): Resolution {
  const configured = setting?.trim();
  if (configured) {
    const expanded = expandPath(configured, env.homeDir, workspaceDir);
    return env.isFile(expanded)
      ? { kind: "found", path: expanded, source: "setting" }
      : { kind: "missing-setting", path: expanded };
  }
  const onPath = findOnPath(tool, env);
  if (onPath) return { kind: "found", path: onPath, source: "path" };
  const downloaded = installedBinary(storageDir, tool, env);
  if (downloaded) return { kind: "found", path: downloaded, source: "download" };
  return { kind: "not-found" };
}

/** Expand `~` and `${workspaceFolder}`; resolve relative paths against the workspace. */
export function expandPath(p: string, homeDir: string, workspaceDir?: string): string {
  let out = p;
  if (workspaceDir) out = out.replace(/\$\{workspaceFolder\}/g, workspaceDir);
  if (out === "~" || out.startsWith("~/") || out.startsWith("~\\")) out = homeDir + out.slice(1);
  if (!path.isAbsolute(out) && !/^[A-Za-z]:[\\/]/.test(out) && workspaceDir) {
    out = path.join(workspaceDir, out);
  }
  return out;
}
