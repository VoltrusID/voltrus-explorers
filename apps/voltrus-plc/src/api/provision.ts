// WebSerial ESP32 provisioning helpers (PLC-26).
//
// Pure logic only: browser-capability detection, the runtime image manifest
// (fetch + validate), pairing-token generation using the exact encodings the
// cloud worker uses (apps/voltrus-plc-cloud/src/routes/relay.ts), and the
// flash plan for the manifest image. Serial flashing lives in
// ProvisionPanel, which dynamically imports esptool-js so non-Chromium
// bundles never load it.

import { CloudApiError, cloudApiBase } from "./cloud-auth";

// Placeholder release channel; overridable via localStorage for testing.
export const DEFAULT_PROVISION_BASE = "https://releases.voltrus.id/esp32-runtime";
export const PROVISION_BASE_KEY = "vplc_provision_base";

// Standard ESP32-S3 app partition offset (partition table 0x8000 + one 0x8000
// slot). The manifest ships one monolithic app image, so the plan is a single
// entry until the runtime splits bootloader/partition/app images.
export const APP_IMAGE_OFFSET = 0x10000;

export class ProvisionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProvisionError";
  }
}

export interface RuntimeManifest {
  version: string;
  chip: "esp32s3";
  url: string;
  sha256: string;
  size: number;
}

export interface FlashPlanEntry {
  address: number;
  url: string;
  sha256: string;
  size: number;
}

export interface RegisteredDevice {
  id: string;
  name: string;
  pairingToken: string;
  createdAt: string;
}

export function isWebSerialAvailable(): boolean {
  return typeof navigator !== "undefined" && "serial" in navigator;
}

export function provisionBase(): string {
  const override = localStorage.getItem(PROVISION_BASE_KEY);
  const base = (override && override.trim()) || DEFAULT_PROVISION_BASE;
  return base.replace(/\/+$/, "");
}

const SHA256_HEX = /^[0-9a-f]{64}$/;

/** Validate an untrusted manifest payload; throws ProvisionError listing every problem. */
export function parseManifest(value: unknown): RuntimeManifest {
  const problems: string[] = [];
  const m = value as Partial<RuntimeManifest> | null;
  if (!m || typeof m !== "object") {
    throw new ProvisionError("manifest is not an object");
  }
  if (typeof m.version !== "string" || m.version.trim().length === 0) {
    problems.push("version must be a non-empty string");
  }
  if (m.chip !== "esp32s3") {
    problems.push(`chip must be "esp32s3" (got ${JSON.stringify(m.chip)})`);
  }
  if (typeof m.url !== "string" || !URL.canParse(m.url)) {
    problems.push("url must be an absolute URL");
  } else if (!/^https?:$/.test(new URL(m.url).protocol)) {
    problems.push("url must be http(s)");
  }
  if (typeof m.sha256 !== "string" || !SHA256_HEX.test(m.sha256)) {
    problems.push("sha256 must be 64 lowercase hex chars");
  }
  if (typeof m.size !== "number" || !Number.isInteger(m.size) || m.size <= 0) {
    problems.push("size must be a positive integer");
  }
  if (problems.length > 0) throw new ProvisionError(`invalid manifest: ${problems.join("; ")}`);
  return { version: m.version!.trim(), chip: "esp32s3", url: m.url!, sha256: m.sha256!, size: m.size! };
}

export async function fetchManifest(base = provisionBase()): Promise<RuntimeManifest> {
  let res: Response;
  try {
    res = await fetch(`${base}/manifest.json`);
  } catch (e) {
    throw new ProvisionError(`manifest fetch failed: ${e instanceof Error ? e.message : String(e)}`);
  }
  if (!res.ok) throw new ProvisionError(`manifest fetch failed: HTTP ${res.status}`);
  return parseManifest(await res.json());
}

function toHex(bytes: Uint8Array): string {
  let hex = "";
  for (const byte of bytes) hex += byte.toString(16).padStart(2, "0");
  return hex;
}

function toBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

// Same encoding as the worker's sha256Hex: SHA-256, lowercase two-digit hex.
// Accepts UTF-8 text (token verification) or raw bytes (image verification).
export async function sha256Hex(input: string | Uint8Array): Promise<string> {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : input;
  const digest = await crypto.subtle.digest("SHA-256", bytes as unknown as ArrayBuffer);
  return toHex(new Uint8Array(digest));
}

// Format-identical to the worker's POST /api/devices token: 32 random bytes,
// base64url without padding (43 chars). The worker issues its own token on
// registration; this exists for tests and any future client-supplied flow.
export function generatePairingToken(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return toBase64Url(bytes);
}

/** Flash plan for a validated manifest: currently one app image at 0x10000. */
export function buildFlashPlan(manifest: RuntimeManifest): FlashPlanEntry[] {
  if (manifest.chip !== "esp32s3") {
    throw new ProvisionError(`no flash plan for chip ${manifest.chip}`);
  }
  return [{ address: APP_IMAGE_OFFSET, url: manifest.url, sha256: manifest.sha256, size: manifest.size }];
}

// Device registration via the cloud REST API, mirroring cloud-auth.ts request
// conventions (Bearer session token, JSON error bodies as CloudApiError).
export async function registerDevice(token: string, name: string): Promise<RegisteredDevice> {
  let res: Response;
  try {
    res = await fetch(`${cloudApiBase()}/api/devices`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify({ name }),
    });
  } catch (e) {
    throw new CloudApiError(0, e instanceof Error ? e.message : String(e));
  }
  if (!res.ok) {
    let message = `HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { error?: unknown };
      if (typeof body.error === "string") message = body.error;
    } catch {
      // non-JSON error body — keep the HTTP status fallback
    }
    throw new CloudApiError(res.status, message);
  }
  const body = (await res.json()) as Partial<RegisteredDevice>;
  if (typeof body.id !== "string" || typeof body.pairingToken !== "string") {
    throw new ProvisionError("device registration response missing id/pairingToken");
  }
  return {
    id: body.id,
    name: typeof body.name === "string" ? body.name : name,
    pairingToken: body.pairingToken,
    createdAt: typeof body.createdAt === "string" ? body.createdAt : new Date().toISOString(),
  };
}

/** Download the runtime image and enforce both size and sha256 from the manifest. */
export async function downloadVerifiedImage(
  entry: FlashPlanEntry,
  onProgress?: (loaded: number, total: number) => void,
): Promise<Uint8Array> {
  let res: Response;
  try {
    res = await fetch(entry.url);
  } catch (e) {
    throw new ProvisionError(`image download failed: ${e instanceof Error ? e.message : String(e)}`);
  }
  if (!res.ok) throw new ProvisionError(`image download failed: HTTP ${res.status}`);

  const chunks: Uint8Array[] = [];
  let loaded = 0;
  if (res.body) {
    const reader = res.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (value) {
        chunks.push(value);
        loaded += value.length;
        onProgress?.(loaded, entry.size);
      }
    }
  } else {
    const buf = await res.arrayBuffer();
    chunks.push(new Uint8Array(buf));
    loaded = buf.byteLength;
    onProgress?.(loaded, entry.size);
  }
  const image = new Uint8Array(loaded);
  let offset = 0;
  for (const chunk of chunks) {
    image.set(chunk, offset);
    offset += chunk.length;
  }
  if (image.length !== entry.size) {
    throw new ProvisionError(`image size mismatch: expected ${entry.size} bytes, got ${image.length}`);
  }
  const actual = await sha256Hex(image);
  if (actual !== entry.sha256) {
    throw new ProvisionError(`image sha256 mismatch: expected ${entry.sha256}, got ${actual}`);
  }
  return image;
}

// Honest fallback text for browsers without Web Serial (Firefox/Safari) and
// for the NVS step, which esptool-js cannot perform in-browser (it exposes raw
// flash writes only — no NVS partition-image generation).
export function cliFallbackCommand(): string {
  return `esptool.py --chip esp32s3 write_flash 0x${APP_IMAGE_OFFSET.toString(16)} <runtime.bin>`;
}
