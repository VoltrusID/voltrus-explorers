import { describe, it, expect, beforeAll, vi } from "vitest";
import { createHash, webcrypto as nodeCrypto } from "node:crypto";
import type { RuntimeManifest } from "../api/provision";

// Node ≥22's localStorage global shadows jsdom's (see cloud-components.test.tsx).
function memStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (k) => map.get(k) ?? null,
    key: (i) => [...map.keys()][i] ?? null,
    removeItem: (k) => {
      map.delete(k);
    },
    setItem: (k, v) => {
      map.set(k, String(v));
    },
  };
}

vi.stubGlobal("localStorage", memStorage());

const {
  parseManifest,
  fetchManifest,
  buildFlashPlan,
  generatePairingToken,
  sha256Hex,
  registerDevice,
  downloadVerifiedImage,
  provisionBase,
  isWebSerialAvailable,
  cliFallbackCommand,
  ProvisionError,
} = await import("../api/provision");
const { CloudApiError } = await import("../api/cloud-auth");

const VALID_MANIFEST = {
  version: "1.0.0",
  chip: "esp32s3",
  url: "https://releases.voltrus.id/esp32-runtime/voltrus-runtime-1.0.0.bin",
  sha256: "a".repeat(64),
  size: 1_048_576,
};

// The worker (apps/voltrus-plc-cloud/src/routes/relay.ts) hashes the UTF-8
// token text with SHA-256 and hex-encodes each byte lowercase, two digits.
// Replicate that here so drift between client and worker formats fails loudly.
function workerSha256Hex(text: string): string {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

function fakeJsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

function fakeBinaryResponse(bytes: Uint8Array): Response {
  return {
    ok: true,
    status: 200,
    body: null,
    arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
  } as unknown as Response;
}

beforeAll(() => {
  // Hashing requires WebCrypto; stub Node's webcrypto when the environment lacks it.
  if (!globalThis.crypto?.subtle) vi.stubGlobal("crypto", nodeCrypto);
});

describe("pairing token generation", () => {
  it("matches the worker format: 32 bytes, base64url, unpadded 43 chars", () => {
    const token = generatePairingToken();
    expect(token).toHaveLength(43);
    expect(token).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it("is unique across calls", () => {
    const seen = new Set(Array.from({ length: 50 }, () => generatePairingToken()));
    expect(seen.size).toBe(50);
  });

  it("hashes to exactly what the worker stores", async () => {
    const token = generatePairingToken();
    expect(await sha256Hex(token)).toBe(workerSha256Hex(token));
  });

  it("matches the SHA-256 test vector for a known string", async () => {
    expect(await sha256Hex("abc")).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });
});

describe("manifest validation", () => {
  it("accepts a valid manifest and trims the version", () => {
    const m = parseManifest({ ...VALID_MANIFEST, version: "  1.2.3  " });
    expect(m).toEqual({ ...VALID_MANIFEST, version: "1.2.3" });
  });

  it("rejects a wrong chip", () => {
    expect(() => parseManifest({ ...VALID_MANIFEST, chip: "esp32" })).toThrow(ProvisionError);
    expect(() => parseManifest({ ...VALID_MANIFEST, chip: "esp32" })).toThrow(/chip/);
  });

  it("rejects a missing sha256", () => {
    const { sha256: _omit, ...withoutSha } = VALID_MANIFEST;
    expect(() => parseManifest(withoutSha)).toThrow(/sha256/);
  });

  it("rejects a non-hex or uppercase sha256", () => {
    expect(() => parseManifest({ ...VALID_MANIFEST, sha256: "z".repeat(64) })).toThrow(/sha256/);
    expect(() => parseManifest({ ...VALID_MANIFEST, sha256: "A".repeat(64) })).toThrow(/sha256/);
  });

  it("rejects bad sizes and urls with all problems listed", () => {
    try {
      parseManifest({ ...VALID_MANIFEST, size: 0, url: "not-a-url" });
      expect.unreachable();
    } catch (e) {
      expect(e).toBeInstanceOf(ProvisionError);
      expect((e as Error).message).toContain("size");
      expect((e as Error).message).toContain("url");
    }
  });

  it("rejects non-objects", () => {
    expect(() => parseManifest(null)).toThrow(ProvisionError);
    expect(() => parseManifest("manifest")).toThrow(ProvisionError);
  });
});

describe("flash plan", () => {
  it("places the app image at 0x10000", () => {
    const plan = buildFlashPlan(parseManifest(VALID_MANIFEST));
    expect(plan).toEqual([
      { address: 0x10000, url: VALID_MANIFEST.url, sha256: VALID_MANIFEST.sha256, size: VALID_MANIFEST.size },
    ]);
  });

  it("refuses non-esp32s3 chips", () => {
    const wrongChip = { ...VALID_MANIFEST, chip: "esp32" } as unknown as RuntimeManifest;
    expect(() => buildFlashPlan(wrongChip)).toThrow(ProvisionError);
  });
});

describe("provision base", () => {
  it("defaults to the release channel", () => {
    expect(provisionBase()).toBe("https://releases.voltrus.id/esp32-runtime");
  });

  it("honours the localStorage override and strips trailing slashes", () => {
    localStorage.setItem("vplc_provision_base", "http://localhost:8788/releases/");
    expect(provisionBase()).toBe("http://localhost:8788/releases");
    localStorage.removeItem("vplc_provision_base");
  });

  it("falls back to the default for blank overrides", () => {
    localStorage.setItem("vplc_provision_base", "   ");
    expect(provisionBase()).toBe("https://releases.voltrus.id/esp32-runtime");
    localStorage.removeItem("vplc_provision_base");
  });
});

describe("manifest fetch", () => {
  it("fetches manifest.json from the base and validates it", async () => {
    const fetchMock = vi.fn().mockResolvedValue(fakeJsonResponse(VALID_MANIFEST));
    vi.stubGlobal("fetch", fetchMock);
    const m = await fetchManifest("http://test-base");
    expect(m.chip).toBe("esp32s3");
    expect(fetchMock).toHaveBeenCalledWith("http://test-base/manifest.json");
  });

  it("surfaces HTTP failures as ProvisionError", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(fakeJsonResponse({ error: "x" }, 404)));
    await expect(fetchManifest("http://test-base")).rejects.toBeInstanceOf(ProvisionError);
  });
});

describe("device registration", () => {
  it("POSTs the Bearer session token and returns the pairing token", async () => {
    const device = { id: "dev-1", name: "Line 1 PLC", pairingToken: "tok_abc", createdAt: "2026-09-24T00:00:00Z" };
    const fetchMock = vi.fn().mockResolvedValue(fakeJsonResponse(device, 201));
    vi.stubGlobal("fetch", fetchMock);
    const got = await registerDevice("session-token", "Line 1 PLC");
    expect(got).toEqual(device);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://localhost:8787/api/devices");
    expect((init.headers as Record<string, string>).Authorization).toBe("Bearer session-token");
    expect(JSON.parse(String(init.body))).toEqual({ name: "Line 1 PLC" });
  });

  it("maps error bodies to CloudApiError", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(fakeJsonResponse({ error: "unauthorized" }, 401)));
    await expect(registerDevice("bad", "dev")).rejects.toBeInstanceOf(CloudApiError);
  });
});

describe("image download", () => {
  it("accepts an image matching size and sha256", async () => {
    const image = new Uint8Array([1, 2, 3, 4]);
    const sha = workerSha256HexBytes(image);
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(fakeBinaryResponse(image)),
    );
    const got = await downloadVerifiedImage({ address: 0x10000, url: "http://img", sha256: sha, size: 4 });
    expect(got).toEqual(image);
  });

  it("rejects size mismatches", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(fakeBinaryResponse(new Uint8Array(3))));
    await expect(
      downloadVerifiedImage({ address: 0x10000, url: "http://img", sha256: "a".repeat(64), size: 4 }),
    ).rejects.toThrow(/size mismatch/);
  });

  it("rejects sha256 mismatches", async () => {
    const image = new Uint8Array([9, 9, 9]);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(fakeBinaryResponse(image)));
    await expect(
      downloadVerifiedImage({ address: 0x10000, url: "http://img", sha256: "a".repeat(64), size: 3 }),
    ).rejects.toThrow(/sha256 mismatch/);
  });
});

function workerSha256HexBytes(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex");
}

describe("browser capability", () => {
  it("reports jsdom as Web Serial-free", () => {
    expect(isWebSerialAvailable()).toBe(false);
  });

  it("reports available once navigator.serial exists", async () => {
    const realNavigator = globalThis.navigator;
    vi.stubGlobal("navigator", { ...realNavigator, serial: { requestPort: () => {} } });
    expect(isWebSerialAvailable()).toBe(true);
    vi.stubGlobal("navigator", realNavigator);
  });

  it("includes the app offset in the CLI fallback", () => {
    expect(cliFallbackCommand()).toContain("0x10000");
    expect(cliFallbackCommand()).toContain("esp32s3");
  });
});
