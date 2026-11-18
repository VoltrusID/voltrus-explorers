/**
 * Bytes of a text file to a string: strict UTF-8 first (a byte order mark is dropped), and
 * Windows-1252 when the bytes are not valid UTF-8. Every byte decodes in Windows-1252, so the
 * fallback never fails.
 */
export function decodeText(bytes: Uint8Array): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return new TextDecoder("windows-1252").decode(bytes);
  }
}
