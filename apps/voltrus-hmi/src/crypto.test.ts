import { describe, it, expect } from "vitest";
import { sha256Hex, stretchPin } from "./crypto";

describe("Pure-JS SHA-256", () => {
  it("matches FIPS 180 test vectors", () => {
    expect(sha256Hex("")).toBe(
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    expect(sha256Hex("abc")).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    expect(sha256Hex("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")).toBe(
      "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    expect(sha256Hex("a".repeat(1000))).toBe(
      "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
    );
  });

  it("is stable across repeated calls (cache not corrupted)", () => {
    const first = sha256Hex("cache-test-1");
    sha256Hex("cache-test-2");
    sha256Hex("cache-test-3");
    expect(sha256Hex("cache-test-1")).toBe(first);
  });
});

describe("PIN stretching", () => {
  it("stretches deterministically and differs per pin", () => {
    const a = stretchPin("00112233445566778899aabbccddeeff", "1234");
    expect(a).toHaveLength(64);
    expect(a).toBe(stretchPin("00112233445566778899aabbccddeeff", "1234"));
    expect(a).not.toBe(stretchPin("00112233445566778899aabbccddeeff", "1235"));
    expect(a).not.toBe(stretchPin("ffeeddccbbaa99887766554433221100", "1234"));
  });
});
