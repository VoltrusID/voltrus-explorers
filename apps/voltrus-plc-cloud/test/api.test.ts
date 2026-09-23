import { describe, expect, it } from "vitest";
import { env, SELF } from "cloudflare:test";

const JSON_HEADERS = { "Content-Type": "application/json" };

function post(path: string, body: unknown, headers: Record<string, string> = {}) {
  return SELF.fetch("https://example.com" + path, {
    method: "POST",
    headers: { ...JSON_HEADERS, ...headers },
    body: JSON.stringify(body),
  });
}

function authed(token: string, init: RequestInit & { path: string }) {
  const { path, ...rest } = init;
  const headers = new Headers(rest.headers);
  headers.set("Authorization", `Bearer ${token}`);
  return SELF.fetch("https://example.com" + path, { ...rest, headers });
}

async function registerUser(email: string): Promise<{ token: string; userId: string }> {
  const res = await post("/api/auth/register", { email, password: "correct horse battery" });
  expect(res.status).toBe(201);
  const body = (await res.json()) as { token: string; user: { id: string } };
  return { token: body.token, userId: body.user.id };
}

describe("GET /api/health", () => {
  it("returns ok without auth", async () => {
    const res = await SELF.fetch("https://example.com/api/health");
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true });
  });
});

describe("auth", () => {
  it("registers, logs in, and reads /me", async () => {
    const email = `reg-${crypto.randomUUID()}@example.com`;
    const { token, userId } = await registerUser(email);

    const login = await post("/api/auth/login", { email, password: "correct horse battery" });
    expect(login.status).toBe(200);
    const loginBody = (await login.json()) as { token: string; user: { id: string; email: string } };
    expect(loginBody.user.id).toBe(userId);
    expect(loginBody.user.email).toBe(email);

    const me = await SELF.fetch("https://example.com/api/auth/me", {
      headers: { Authorization: `Bearer ${loginBody.token}` },
    });
    expect(me.status).toBe(200);
    expect(await me.json()).toEqual({ user: { id: userId, email } });
  });

  it("rejects duplicate email with 409", async () => {
    const email = `dup-${crypto.randomUUID()}@example.com`;
    await registerUser(email);
    const res = await post("/api/auth/register", { email, password: "another password" });
    expect(res.status).toBe(409);
  });

  it("rejects invalid email and short password with 400", async () => {
    const badEmail = await post("/api/auth/register", { email: "not-an-email", password: "long enough 123" });
    expect(badEmail.status).toBe(400);
    const shortPassword = await post("/api/auth/register", { email: `x${crypto.randomUUID()}@e.com`, password: "short" });
    expect(shortPassword.status).toBe(400);
  });

  it("rejects bad login credentials with 401", async () => {
    const email = `bad-${crypto.randomUUID()}@example.com`;
    await registerUser(email);
    const wrongPassword = await post("/api/auth/login", { email, password: "definitely wrong" });
    expect(wrongPassword.status).toBe(401);
    const unknownUser = await post("/api/auth/login", { email: `nobody-${crypto.randomUUID()}@e.com`, password: "whatever 123" });
    expect(unknownUser.status).toBe(401);
  });

  it("rejects /me with missing and garbage tokens", async () => {
    const noToken = await SELF.fetch("https://example.com/api/auth/me");
    expect(noToken.status).toBe(401);

    const garbage = await SELF.fetch("https://example.com/api/auth/me", {
      headers: { Authorization: "Bearer not-a-real-token" },
    });
    expect(garbage.status).toBe(401);

    const tampered = await SELF.fetch("https://example.com/api/auth/me", {
      headers: { Authorization: "Bearer eyJ1aWQiOiJ4In0.forged-signature" },
    });
    expect(tampered.status).toBe(401);

    // Invalid base64 in the signature segment must 401, never escape as a 500.
    const malformedBase64 = await SELF.fetch("https://example.com/api/auth/me", {
      headers: { Authorization: "Bearer eyJ1aWQiOiJ4In0.%zz" },
    });
    expect(malformedBase64.status).toBe(401);
  });
});

describe("projects and files", () => {
  it("covers the full happy path: create, write, read, list, detail, delete", async () => {
    const { token, userId } = await registerUser(`flow-${crypto.randomUUID()}@example.com`);

    const created = await post("/api/projects", { name: "Line 3 PLC" }, { Authorization: `Bearer ${token}` });
    expect(created.status).toBe(201);
    const project = (await created.json()) as { id: string; name: string; createdAt: string; updatedAt: string };
    expect(project.name).toBe("Line 3 PLC");
    expect(project.id).toBeTruthy();

    const putA = await authed(token, {
      path: `/api/projects/${project.id}/files/ladder.txt`,
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify({ content: "|--[ ]--( )--|" }),
    });
    expect(putA.status).toBe(204);

    const putNested = await authed(token, {
      path: `/api/projects/${project.id}/files/src/main.rs`,
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify({ content: "fn main() {}" }),
    });
    expect(putNested.status).toBe(204);

    const list = await authed(token, { path: `/api/projects/${project.id}/files`, method: "GET" });
    expect(list.status).toBe(200);
    const files = (await list.json()) as Array<{ path: string; size: number; updatedAt: string }>;
    expect(files.map((f) => f.path)).toEqual(["ladder.txt", "src/main.rs"]);
    // size is byte length, not JS string length (both ASCII here: 14 and 11)
    expect(files.find((f) => f.path === "ladder.txt")?.size).toBe(14);

    const get = await authed(token, { path: `/api/projects/${project.id}/files/src/main.rs`, method: "GET" });
    expect(get.status).toBe(200);
    expect(await get.json()).toEqual({ content: "fn main() {}" });

    const detail = await authed(token, { path: `/api/projects/${project.id}`, method: "GET" });
    expect(detail.status).toBe(200);
    const detailBody = (await detail.json()) as { id: string; files: Array<{ path: string }> };
    expect(detailBody.id).toBe(project.id);
    expect(detailBody.files).toHaveLength(2);

    const index = await authed(token, { path: "/api/projects", method: "GET" });
    expect(index.status).toBe(200);
    const projects = (await index.json()) as Array<{ id: string; fileCount: number }>;
    expect(projects.find((p) => p.id === project.id)?.fileCount).toBe(2);

    const del = await authed(token, { path: `/api/projects/${project.id}`, method: "DELETE" });
    expect(del.status).toBe(204);

    const afterDelete = await authed(token, { path: `/api/projects/${project.id}`, method: "GET" });
    expect(afterDelete.status).toBe(404);

    // R2 keys for the deleted project are gone too.
    const r2Keys = await listR2Keys(`${userId}/${project.id}/`);
    expect(r2Keys).toEqual([]);
  });

  it("keeps file metadata in sync on overwrite and delete", async () => {
    const { token } = await registerUser(`sync-${crypto.randomUUID()}@example.com`);
    const project = await createProject(token, "Sync");

    const put = (content: string) =>
      authed(token, {
        path: `/api/projects/${project.id}/files/a.txt`,
        method: "PUT",
        headers: JSON_HEADERS,
        body: JSON.stringify({ content }),
      });

    await put("first");
    await put("second version"); // overwrite: upsert, no duplicate row

    const list = await authed(token, { path: `/api/projects/${project.id}/files`, method: "GET" });
    const files = (await list.json()) as Array<{ path: string; size: number }>;
    expect(files).toHaveLength(1);
    expect(files[0]!.size).toBe("second version".length);

    const del = await authed(token, { path: `/api/projects/${project.id}/files/a.txt`, method: "DELETE" });
    expect(del.status).toBe(204);
    const after = await authed(token, { path: `/api/projects/${project.id}/files`, method: "GET" });
    expect(await after.json()).toEqual([]);
  });

  it("rejects invalid file paths with 400", async () => {
    const { token } = await registerUser(`paths-${crypto.randomUUID()}@example.com`);
    const project = await createProject(token, "Paths");

    // Vector notes: fetch's URL parser resolves dot segments ("..", "%2E%2E")
    // before the request is issued, so those forms cannot reach the worker.
    // These vectors survive normalization yet still carry the forbidden
    // substrings — exercising the server-side validator directly.
    for (const bad of ["%2E%2Ehidden", "a%2F..b", "%2Fabsolute", "nul%00path", "a".repeat(600)]) {
      const res = await authed(token, {
        path: `/api/projects/${project.id}/files/${bad}`,
        method: "PUT",
        headers: JSON_HEADERS,
        body: JSON.stringify({ content: "x" }),
      });
      expect(res.status, `expected 400 for path ${bad}`).toBe(400);
    }
  });

  it("returns 404 (not 403) for foreign projects and files", async () => {
    const owner = await registerUser(`owner-${crypto.randomUUID()}@example.com`);
    const stranger = await registerUser(`stranger-${crypto.randomUUID()}@example.com`);
    const project = await createProject(owner.token, "Owner Only");
    await authed(owner.token, {
      path: `/api/projects/${project.id}/files/private.txt`,
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify({ content: "secret" }),
    });

    const foreignDetail = await authed(stranger.token, { path: `/api/projects/${project.id}`, method: "GET" });
    expect(foreignDetail.status).toBe(404);

    const foreignRead = await authed(stranger.token, {
      path: `/api/projects/${project.id}/files/private.txt`,
      method: "GET",
    });
    expect(foreignRead.status).toBe(404);

    const foreignWrite = await authed(stranger.token, {
      path: `/api/projects/${project.id}/files/hack.txt`,
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify({ content: "nope" }),
    });
    expect(foreignWrite.status).toBe(404);

    const foreignDelete = await authed(stranger.token, { path: `/api/projects/${project.id}`, method: "DELETE" });
    expect(foreignDelete.status).toBe(404);
    // Owner still sees the project untouched.
    const ownerView = await authed(owner.token, { path: `/api/projects/${project.id}`, method: "GET" });
    expect(ownerView.status).toBe(200);
  });

  it("requires auth on project and file routes", async () => {
    const noToken = await SELF.fetch("https://example.com/api/projects");
    expect(noToken.status).toBe(401);

    const garbage = await authed("garbage-token", { path: "/api/projects", method: "GET" });
    expect(garbage.status).toBe(401);

    const noTokenFile = await SELF.fetch("https://example.com/api/projects/any-id/files/a.txt", { method: "PUT" });
    expect(noTokenFile.status).toBe(401);
  });

  it("rejects malformed project create payloads", async () => {
    const { token } = await registerUser(`badproj-${crypto.randomUUID()}@example.com`);
    const noName = await post("/api/projects", {}, { Authorization: `Bearer ${token}` });
    expect(noName.status).toBe(400);
    const emptyName = await post("/api/projects", { name: "   " }, { Authorization: `Bearer ${token}` });
    expect(emptyName.status).toBe(400);
  });
});

describe("CORS", () => {
  it("answers preflight for allowed origins with 204", async () => {
    const res = await SELF.fetch("https://example.com/api/projects", {
      method: "OPTIONS",
      headers: { Origin: "http://localhost:1440", "Access-Control-Request-Method": "PUT" },
    });
    expect(res.status).toBe(204);
    expect(res.headers.get("Access-Control-Allow-Origin")).toBe("http://localhost:1440");
    expect(res.headers.get("Access-Control-Allow-Methods")).toContain("PUT");
    expect(res.headers.get("Access-Control-Allow-Headers")).toContain("Authorization");
  });

  it("echoes allowed origin on real responses and withholds it for strangers", async () => {
    // Same-origin SPA deployment moved the prod origin to app.plc.voltrus.id
    // (wrangler.toml CORS_ORIGINS); localhost stays for vite dev.
    const allowed = await SELF.fetch("https://example.com/api/health", {
      headers: { Origin: "https://app.plc.voltrus.id" },
    });
    expect(allowed.headers.get("Access-Control-Allow-Origin")).toBe("https://app.plc.voltrus.id");

    const denied = await SELF.fetch("https://example.com/api/health", {
      headers: { Origin: "https://evil.example" },
    });
    expect(denied.headers.get("Access-Control-Allow-Origin")).toBeNull();
  });
});

async function createProject(token: string, name: string): Promise<{ id: string }> {
  const res = await post("/api/projects", { name }, { Authorization: `Bearer ${token}` });
  expect(res.status).toBe(201);
  return (await res.json()) as { id: string };
}

async function listR2Keys(prefix: string): Promise<string[]> {
  const listed = await env.FILES.list({ prefix });
  return listed.objects.map((o) => o.key);
}
