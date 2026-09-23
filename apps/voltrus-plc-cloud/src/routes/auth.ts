import { Hono } from "hono";
import { authMiddleware, hashPassword, issueToken, verifyPassword } from "../auth";
import type { AppEnv } from "../env";

export const authRoutes = new Hono<AppEnv>();

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

type Credentials = { email: string; password: string };

async function readCredentials(c: {
  req: { json: () => Promise<unknown> };
}): Promise<Credentials | null> {
  const body = await c.req.json().catch(() => null);
  if (!body || typeof body !== "object") return null;
  const { email, password } = body as Record<string, unknown>;
  if (typeof email !== "string" || typeof password !== "string") return null;
  return { email: email.trim().toLowerCase(), password };
}

authRoutes.post("/register", async (c) => {
  const creds = await readCredentials(c);
  if (!creds) return c.json({ error: "email and password are required" }, 400);
  if (!EMAIL_RE.test(creds.email)) return c.json({ error: "invalid email" }, 400);
  if (creds.password.length < 8) return c.json({ error: "password must be at least 8 characters" }, 400);

  const existing = await c.env.DB.prepare("SELECT id FROM users WHERE email = ?").bind(creds.email).first();
  if (existing) return c.json({ error: "email already registered" }, 409);

  const id = crypto.randomUUID();
  const { hash, salt } = await hashPassword(creds.password);
  const now = new Date().toISOString();
  try {
    await c.env.DB.prepare("INSERT INTO users (id, email, password_hash, salt, created_at) VALUES (?, ?, ?, ?, ?)")
      .bind(id, creds.email, hash, salt, now)
      .run();
  } catch {
    // UNIQUE(email) constraint lost the race with a concurrent register.
    return c.json({ error: "email already registered" }, 409);
  }

  const token = await issueToken(c.env.JWT_SECRET, id);
  return c.json({ token, user: { id, email: creds.email } }, 201);
});

authRoutes.post("/login", async (c) => {
  const creds = await readCredentials(c);
  if (!creds) return c.json({ error: "email and password are required" }, 400);

  const row = await c.env.DB.prepare("SELECT id, email, password_hash, salt FROM users WHERE email = ?")
    .bind(creds.email)
    .first<{ id: string; email: string; password_hash: string; salt: string }>();
  const ok = row ? await verifyPassword(creds.password, row.password_hash, row.salt) : false;
  if (!row || !ok) return c.json({ error: "invalid credentials" }, 401);

  const token = await issueToken(c.env.JWT_SECRET, row.id);
  return c.json({ token, user: { id: row.id, email: row.email } });
});

authRoutes.get("/me", authMiddleware, async (c) => {
  const row = await c.env.DB.prepare("SELECT id, email FROM users WHERE id = ?")
    .bind(c.get("userId"))
    .first<{ id: string; email: string }>();
  if (!row) return c.json({ error: "user not found" }, 404);
  return c.json({ user: { id: row.id, email: row.email } });
});
