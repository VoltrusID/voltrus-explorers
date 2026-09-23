// Password hashing + session token primitives — Web Crypto only, workers-runtime safe.
import type { Context, Next } from "hono";
import type { AppEnv } from "./env";

const PBKDF2_ITERATIONS = 100_000;
const TOKEN_TTL_SECONDS = 7 * 24 * 60 * 60;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function toHex(bytes: Uint8Array): string {
  let hex = "";
  for (const byte of bytes) hex += byte.toString(16).padStart(2, "0");
  return hex;
}

function fromHex(hex: string): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i += 1) bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return bytes;
}

function toBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function fromBase64Url(encoded: string): Uint8Array<ArrayBuffer> {
  const padded = encoded.replaceAll("-", "+").replaceAll("_", "/");
  const rest = padded.length % 4;
  const normalized = rest === 0 ? padded : padded + "=".repeat(4 - rest);
  const binary = atob(normalized);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

async function deriveBits(password: string, salt: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(password) as unknown as ArrayBuffer,
    "PBKDF2",
    false,
    ["deriveBits"],
  );
  const bits = await crypto.subtle.deriveBits(
    { name: "PBKDF2", hash: "SHA-256", salt: salt as unknown as ArrayBuffer, iterations: PBKDF2_ITERATIONS },
    key,
    256,
  );
  return new Uint8Array(bits);
}

export async function hashPassword(password: string): Promise<{ hash: string; salt: string }> {
  const salt = new Uint8Array(16);
  crypto.getRandomValues(salt);
  const hash = await deriveBits(password, salt);
  return { hash: toHex(hash), salt: toHex(salt) };
}

// Length-checked constant-time comparison via XOR accumulation.
function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i += 1) diff |= a[i]! ^ b[i]!;
  return diff === 0;
}

export async function verifyPassword(password: string, hashHex: string, saltHex: string): Promise<boolean> {
  const derived = await deriveBits(password, fromHex(saltHex));
  return bytesEqual(derived, fromHex(hashHex));
}

async function hmac(secret: string, payload: string): Promise<Uint8Array<ArrayBuffer>> {
  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(secret) as unknown as ArrayBuffer,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, encoder.encode(payload) as unknown as ArrayBuffer);
  return new Uint8Array(sig);
}

/** `base64url(JSON {uid, exp}) + "." + base64url(HMAC-SHA256(secret, payload))`, exp = now + 7d. */
export async function issueToken(secret: string, uid: string): Promise<string> {
  const payload = JSON.stringify({ uid, exp: Math.floor(Date.now() / 1000) + TOKEN_TTL_SECONDS });
  const encoded = toBase64Url(encoder.encode(payload));
  const sig = await hmac(secret, encoded);
  return `${encoded}.${toBase64Url(sig)}`;
}

/** Returns the uid for a valid, unexpired token, else null. */
export async function verifyToken(secret: string, token: string): Promise<string | null> {
  const parts = token.split(".");
  if (parts.length !== 2 || !parts[0] || !parts[1]) return null;
  const [payload, sig] = parts;
  const expected = await hmac(secret, payload);
  let sigBytes: Uint8Array<ArrayBuffer>;
  try {
    sigBytes = fromBase64Url(sig);
  } catch {
    // Malformed base64 in the attacker-controlled header must be 401, never a thrown 500.
    return null;
  }
  if (!bytesEqual(sigBytes, expected)) return null;
  let claims: { uid?: unknown; exp?: unknown };
  try {
    claims = JSON.parse(decoder.decode(fromBase64Url(payload)));
  } catch {
    return null;
  }
  if (typeof claims.uid !== "string" || typeof claims.exp !== "number") return null;
  if (claims.exp <= Math.floor(Date.now() / 1000)) return null;
  return claims.uid;
}

export async function authMiddleware(c: Context<AppEnv>, next: Next) {
  const header = c.req.header("Authorization") ?? "";
  const match = /^Bearer\s+(.+)$/.exec(header);
  const userId = match ? await verifyToken(c.env.JWT_SECRET, match[1]!) : null;
  if (!userId) return c.json({ error: "unauthorized" }, 401);
  c.set("userId", userId);
  await next();
}
