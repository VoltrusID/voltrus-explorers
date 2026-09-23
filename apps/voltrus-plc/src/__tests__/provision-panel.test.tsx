import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// Hoisted mocks: the panel is tested against a stubbed provision module so
// no real fetch/crypto/serial runs here.
const { fetchManifestMock, registerDeviceMock, serialAvailableMock } = vi.hoisted(() => ({
  fetchManifestMock: vi.fn(),
  registerDeviceMock: vi.fn(),
  serialAvailableMock: vi.fn(),
}));

vi.mock("../api/provision", () => ({
  APP_IMAGE_OFFSET: 0x10000,
  PROVISION_BASE_KEY: "vplc_provision_base",
  buildFlashPlan: (m: { url: string; sha256: string; size: number }) => [
    { address: 0x10000, url: m.url, sha256: m.sha256, size: m.size },
  ],
  cliFallbackCommand: () => "esptool.py --chip esp32s3 write_flash 0x10000 <runtime.bin>",
  downloadVerifiedImage: vi.fn(),
  fetchManifest: fetchManifestMock,
  isWebSerialAvailable: serialAvailableMock,
  provisionBase: () => "http://test-api",
  registerDevice: registerDeviceMock,
}));

// store/cloud.ts imports the bridge at module load; stub it like the other
// cloud component tests do. registerCloudCommands covers the relay-bridge
// side-effect import chain.
vi.mock("../api/bridge", () => ({
  invoke: vi.fn(),
  listen: vi.fn().mockResolvedValue(() => {}),
  registerCloudCommands: vi.fn(),
  BridgeError: class BridgeError extends Error {
    constructor(
      public code: string,
      public cmd: string,
    ) {
      super(`${code} (${cmd})`);
    }
  },
}));

// Node ≥22's localStorage global is undefined without --localstorage-file and
// shadows jsdom's; installed before the (dynamic) imports below.
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

const { useCloudStore } = await import("../store/cloud");
const { default: ProvisionPanel } = await import("../components/Cloud/ProvisionPanel");

const MANIFEST = {
  version: "1.0.0",
  chip: "esp32s3" as const,
  url: "https://releases.voltrus.id/esp32-runtime/voltrus-runtime-1.0.0.bin",
  sha256: "a".repeat(64),
  size: 1_048_576,
};

const DEVICE = {
  id: "dev-1",
  name: "Line 1 PLC",
  pairingToken: "tok-abc-123",
  createdAt: "2026-09-24T00:00:00Z",
};

let container: HTMLElement;
let root: Root;

const renderNode = (node: React.ReactNode) =>
  act(async () => {
    root.render(node);
  });

const click = (el: Element) =>
  act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  });

const setInputValue = (el: Element, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(el, value);
  act(() => {
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

const buttonByText = (text: string) =>
  Array.from(container.querySelectorAll("button")).find((b) => b.textContent === text);

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  useCloudStore.setState({ token: null, user: null });
  fetchManifestMock.mockReset().mockResolvedValue(MANIFEST);
  registerDeviceMock.mockReset();
  serialAvailableMock.mockReset().mockReturnValue(false);
});

afterEach(() => {
  act(() => {
    root.unmount();
  });
  container.remove();
});

const textContent = () => container.textContent ?? "";

describe("ProvisionPanel", () => {
  it("shows CLI fallback instructions when Web Serial is missing", async () => {
    await renderNode(<ProvisionPanel />);
    expect(textContent()).toContain("Web Serial NOT available");
    expect(textContent()).toContain("esptool.py --chip esp32s3 write_flash 0x10000");
    const link = container.querySelector<HTMLAnchorElement>("a[href]");
    expect(link?.getAttribute("href")).toBe(MANIFEST.url);
    // No flash step without serial.
    expect(buttonByText("Connect & flash")).toBeUndefined();
  });

  it("shows a manifest failure with the override hint", async () => {
    fetchManifestMock.mockRejectedValue(new Error("HTTP 404"));
    await renderNode(<ProvisionPanel />);
    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("Manifest failed: HTTP 404");
    expect(textContent()).toContain("vplc_provision_base");
  });

  it("asks signed-out users to sign in", async () => {
    useCloudStore.setState({ token: null });
    await renderNode(<ProvisionPanel />);
    expect(textContent()).toContain("Sign in to provision devices.");
    expect(container.querySelector('input[aria-label="Device name"]')).toBeNull();
  });

  it("registers a device and reveals the pairing token + flash step", async () => {
    serialAvailableMock.mockReturnValue(true);
    registerDeviceMock.mockResolvedValue(DEVICE);
    useCloudStore.setState({ token: "session-token" });
    await renderNode(<ProvisionPanel />);

    const input = container.querySelector('input[aria-label="Device name"]');
    expect(input).not.toBeNull();
    await setInputValue(input!, "Line 1 PLC");
    const registerBtn = buttonByText("Register");
    expect(registerBtn).toBeDefined();
    await click(registerBtn!);

    expect(registerDeviceMock).toHaveBeenCalledWith("session-token", "Line 1 PLC");
    expect(textContent()).toContain("tok-abc-123");
    expect(textContent()).toContain("shown only once");

    const flashBtn = buttonByText("Connect & flash");
    expect(flashBtn).toBeDefined();
    expect(flashBtn!.disabled).toBe(false);
  });

  it("surfaces registration errors", async () => {
    serialAvailableMock.mockReturnValue(true);
    registerDeviceMock.mockRejectedValue(new Error("HTTP 401"));
    useCloudStore.setState({ token: "session-token" });
    await renderNode(<ProvisionPanel />);

    const input = container.querySelector('input[aria-label="Device name"]')!;
    await setInputValue(input, "Line 1 PLC");
    await click(buttonByText("Register")!);

    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("HTTP 401");
  });
});
