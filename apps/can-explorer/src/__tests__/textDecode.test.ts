import { describe, expect, it } from "vitest";
import { decodeText } from "../../../../packages/mt-ui/src/textDecode";

const bytes = (...b: number[]) => new Uint8Array(b);

describe("decodeText (openTextFile)", () => {
  it("reads UTF-8 and drops a byte order mark", () => {
    expect(decodeText(new TextEncoder().encode('CM_ "Température °C";'))).toBe('CM_ "Température °C";');
    expect(decodeText(bytes(0xef, 0xbb, 0xbf, 0x42, 0x4f, 0x5f))).toBe("BO_");
  });

  it("falls back to Windows-1252 when the bytes are not UTF-8", () => {
    // "Température °C", "Ein/Aus €" as CANdb++ writes them on Windows.
    expect(decodeText(bytes(0x54, 0x65, 0x6d, 0x70, 0xe9, 0x72, 0x61, 0x74, 0x75, 0x72, 0x65, 0x20, 0xb0, 0x43))).toBe("Température °C");
    expect(decodeText(bytes(0x41, 0x75, 0x73, 0x20, 0x80))).toBe("Aus €");
    expect(decodeText(bytes(0xc4, 0xd6, 0xdc))).toBe("ÄÖÜ");
  });

  it("never produces replacement characters", () => {
    const all = new Uint8Array(256).map((_, i) => i);
    expect(decodeText(all)).not.toContain("�");
  });
});
