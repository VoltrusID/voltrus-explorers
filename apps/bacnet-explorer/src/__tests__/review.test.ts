import { describe, expect, it, vi } from "vitest";
import { createReviewGate, MIN_UPTIME_MS, SETTLE_MS, type ReviewDeps } from "../../../../packages/mt-ui/src/review";
import { tagUrl } from "../../../../packages/mt-ui/src/links";

function setup(over: Partial<ReviewDeps> = {}, stored: string | null = null) {
  vi.useFakeTimers();
  vi.setSystemTime(0);
  let storedVersion = stored;
  const request = vi.fn(async () => true);
  const deps: ReviewDeps = {
    now: () => Date.now(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
    isStoreBuild: async () => true,
    version: async () => "1.0.0",
    getStored: () => storedVersion,
    setStored: (v) => {
      storedVersion = v;
    },
    modalOpen: () => false,
    request,
    ...over,
  };
  return { gate: createReviewGate(deps), request, stored: () => storedVersion };
}

const advance = (ms: number) => vi.advanceTimersByTimeAsync(ms);

describe("review gate", () => {
  it("never fires before 30 s of uptime", async () => {
    const t = setup();
    await advance(MIN_UPTIME_MS - 1000);
    t.gate.noteDeviceSuccess();
    await advance(SETTLE_MS * 3);
    expect(t.request).not.toHaveBeenCalled();
  });

  it("fires 2 s after a success past 30 s, persists the version, once per session", async () => {
    const t = setup();
    await advance(MIN_UPTIME_MS);
    t.gate.noteDeviceSuccess();
    await advance(SETTLE_MS - 1);
    expect(t.request).not.toHaveBeenCalled();
    await advance(1);
    expect(t.request).toHaveBeenCalledTimes(1);
    expect(t.stored()).toBe("1.0.0");
    t.gate.noteDeviceSuccess();
    await advance(SETTLE_MS * 2);
    expect(t.request).toHaveBeenCalledTimes(1);
  });

  it("an error during the 2 s wait cancels it; a later quiet success fires", async () => {
    const t = setup();
    await advance(MIN_UPTIME_MS);
    t.gate.noteDeviceSuccess();
    await advance(1000);
    t.gate.noteDeviceError();
    await advance(SETTLE_MS * 2);
    expect(t.request).not.toHaveBeenCalled();
    t.gate.noteDeviceSuccess();
    await advance(SETTLE_MS);
    expect(t.request).toHaveBeenCalledTimes(1);
  });

  it("ignores a success within 2 s after an error", async () => {
    const t = setup();
    await advance(MIN_UPTIME_MS);
    t.gate.noteDeviceError();
    await advance(500);
    t.gate.noteDeviceSuccess();
    await advance(SETTLE_MS * 2);
    expect(t.request).not.toHaveBeenCalled();
  });

  it("skips when this version already asked", async () => {
    const t = setup({}, "1.0.0");
    await advance(MIN_UPTIME_MS);
    t.gate.noteDeviceSuccess();
    await advance(SETTLE_MS);
    expect(t.request).not.toHaveBeenCalled();
  });

  it("asks again for a new version", async () => {
    const t = setup({}, "0.9.0");
    await advance(MIN_UPTIME_MS);
    t.gate.noteDeviceSuccess();
    await advance(SETTLE_MS);
    expect(t.request).toHaveBeenCalledTimes(1);
  });

  it("skips non-store builds and open modals; does not persist when StoreKit was not asked", async () => {
    const a = setup({ isStoreBuild: async () => false });
    await advance(MIN_UPTIME_MS);
    a.gate.noteDeviceSuccess();
    await advance(SETTLE_MS);
    expect(a.request).not.toHaveBeenCalled();
    vi.useRealTimers();

    const b = setup({ modalOpen: () => true });
    await advance(MIN_UPTIME_MS);
    b.gate.noteDeviceSuccess();
    await advance(SETTLE_MS);
    expect(b.request).not.toHaveBeenCalled();
    vi.useRealTimers();

    const c = setup({ request: vi.fn(async () => false) });
    await advance(MIN_UPTIME_MS);
    c.gate.noteDeviceSuccess();
    await advance(SETTLE_MS);
    expect(c.stored()).toBeNull();
    vi.useRealTimers();
  });
});

describe("tagUrl", () => {
  const tag = (u: string) => tagUrl(u, "privacy", "modbus-poll", "macos");
  it("tags voltrus.id and subdomains", () => {
    expect(tag("https://voltrus.id/privacy/")).toBe("https://voltrus.id/privacy/?utm_source=modbus-poll&utm_medium=app&utm_campaign=privacy&utm_content=macos");
    expect(tag("https://console.voltrus.id/app/settings?delete=1")).toBe(
      "https://console.voltrus.id/app/settings?delete=1&utm_source=modbus-poll&utm_medium=app&utm_campaign=privacy&utm_content=macos",
    );
  });
  it("never overwrites existing utm params", () => {
    expect(tag("https://voltrus.id/x?utm_source=mail")).toContain("utm_source=mail&");
    expect(tag("https://voltrus.id/x?utm_source=mail")).not.toContain("modbus-poll");
  });
  it("leaves other hosts alone", () => {
    for (const u of ["https://www.apple.com/legal/", "https://github.com/VoltrusID/voltrus-explorers", "https://notvoltrus.id/", "https://voltrus.id.evil.com/", "not a url"]) expect(tag(u)).toBe(u);
  });
});
