// AES-GCM encryption at rest for provider API keys. The AES key is derived with
// HKDF-SHA-256 from JWT_SECRET and the fixed salt "ai-keys-v1", so rotating
// JWT_SECRET invalidates stored keys — nothing beyond JWT_SECRET to provision.

const KEY_SALT = "ai-keys-v1";
const IV_BYTES = 12;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

let cached: { secret: string; key: CryptoKey } | undefined;

async function derivedKey(secret: string): Promise<CryptoKey> {
  if (cached?.secret === secret) return cached.key;
  const ikm = await crypto.subtle.importKey(
    "raw",
    encoder.encode(secret) as unknown as ArrayBuffer,
    "HKDF",
    false,
    ["deriveKey"],
  );
  const key = await crypto.subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: encoder.encode(KEY_SALT) as unknown as ArrayBuffer, info: new Uint8Array(0) },
    ikm,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
  cached = { secret, key };
  return key;
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(encoded: string): Uint8Array<ArrayBuffer> {
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export async function encryptKey(secret: string, plaintext: string): Promise<{ encrypted: string; iv: string }> {
  const key = await derivedKey(secret);
  const iv = crypto.getRandomValues(new Uint8Array(IV_BYTES));
  const cipher = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: iv as unknown as ArrayBuffer },
    key,
    encoder.encode(plaintext) as unknown as ArrayBuffer,
  );
  return { encrypted: toBase64(new Uint8Array(cipher)), iv: toBase64(iv) };
}

export async function decryptKey(secret: string, encryptedBase64: string, ivBase64: string): Promise<string> {
  const key = await derivedKey(secret);
  const plain = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: fromBase64(ivBase64) as unknown as ArrayBuffer },
    key,
    fromBase64(encryptedBase64) as unknown as ArrayBuffer,
  );
  return decoder.decode(plain);
}

/** `first6...last4` for display; the plaintext never leaves the server. */
export function maskKey(key: string): string {
  return key.slice(0, 6) + "..." + key.slice(-4);
}
