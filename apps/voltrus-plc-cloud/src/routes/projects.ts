import { Hono } from "hono";
import { authMiddleware } from "../auth";
import type { AppEnv } from "../env";
import { effectivePlan, FREE_MAX_PROJECTS } from "../entitlements";

export const projectsRoutes = new Hono<AppEnv>();

projectsRoutes.use("*", authMiddleware);

export type ProjectRow = {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
};

/** Project lookup scoped to the session user — foreign or missing ids are indistinguishable (404). */
export async function getOwnedProject(db: AppEnv["Bindings"]["DB"], userId: string, projectId: string) {
  return db
    .prepare("SELECT id, name, created_at, updated_at FROM projects WHERE id = ? AND user_id = ?")
    .bind(projectId, userId)
    .first<ProjectRow>();
}

projectsRoutes.get("/", async (c) => {
  const { results } = await c.env.DB.prepare(
    `SELECT p.id, p.name, p.created_at, p.updated_at,
       (SELECT COUNT(*) FROM project_files f WHERE f.project_id = p.id) AS fileCount
     FROM projects p WHERE p.user_id = ? ORDER BY p.created_at DESC`,
  )
    .bind(c.get("userId"))
    .all<{ id: string; name: string; created_at: string; updated_at: string; fileCount: number }>();

  return c.json(
    results.map((r) => ({ id: r.id, name: r.name, createdAt: r.created_at, updatedAt: r.updated_at, fileCount: r.fileCount })),
  );
});

projectsRoutes.post("/", async (c) => {
  const body = await c.req.json().catch(() => null);
  const name = body && typeof body === "object" ? (body as Record<string, unknown>).name : undefined;
  if (typeof name !== "string" || name.trim().length === 0) {
    return c.json({ error: "name is required" }, 400);
  }

  // Free tier caps how many projects a user can hold; paid plans are unlimited.
  if ((await effectivePlan(c.env.DB, c.get("userId"))) === "free") {
    const count = await c.env.DB.prepare("SELECT COUNT(*) AS n FROM projects WHERE user_id = ?")
      .bind(c.get("userId"))
      .first<{ n: number }>();
    if ((count?.n ?? 0) >= FREE_MAX_PROJECTS) {
      return c.json({ error: "plan_required", requiredPlan: "solo" }, 402);
    }
  }

  const id = crypto.randomUUID();
  const now = new Date().toISOString();
  await c.env.DB.prepare("INSERT INTO projects (id, user_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
    .bind(id, c.get("userId"), name.trim(), now, now)
    .run();

  return c.json({ id, name: name.trim(), createdAt: now, updatedAt: now }, 201);
});

projectsRoutes.get("/:id", async (c) => {
  const project = await getOwnedProject(c.env.DB, c.get("userId"), c.req.param("id"));
  if (!project) return c.json({ error: "project not found" }, 404);

  const { results } = await c.env.DB.prepare(
    "SELECT path, size, updated_at FROM project_files WHERE project_id = ? ORDER BY path",
  )
    .bind(project.id)
    .all<{ path: string; size: number; updated_at: string }>();

  return c.json({
    id: project.id,
    name: project.name,
    createdAt: project.created_at,
    updatedAt: project.updated_at,
    files: results.map((f) => ({ path: f.path, size: f.size, updatedAt: f.updated_at })),
  });
});

projectsRoutes.delete("/:id", async (c) => {
  const project = await getOwnedProject(c.env.DB, c.get("userId"), c.req.param("id"));
  if (!project) return c.json({ error: "project not found" }, 404);

  // D1 first: if R2 cleanup fails afterwards we leak orphaned bodies (harmless,
  // invisible garbage) instead of metadata rows whose bodies are gone.
  await c.env.DB.batch([
    c.env.DB.prepare("DELETE FROM project_files WHERE project_id = ?").bind(project.id),
    c.env.DB.prepare("DELETE FROM projects WHERE id = ?").bind(project.id),
  ]);

  const prefix = `${c.get("userId")}/${project.id}/`;
  try {
    let cursor: string | undefined;
    const keys: string[] = [];
    do {
      const listed = await c.env.FILES.list({ prefix, cursor });
      keys.push(...listed.objects.map((o) => o.key));
      cursor = listed.truncated ? listed.cursor : undefined;
    } while (cursor);
    for (let i = 0; i < keys.length; i += 1000) {
      await c.env.FILES.delete(keys.slice(i, i + 1000));
    }
  } catch {
    // Best-effort cleanup; the project is already gone from the API surface.
  }

  return c.body(null, 204);
});

projectsRoutes.get("/:id/files", async (c) => {
  const project = await getOwnedProject(c.env.DB, c.get("userId"), c.req.param("id"));
  if (!project) return c.json({ error: "project not found" }, 404);

  const { results } = await c.env.DB.prepare(
    "SELECT path, size, updated_at FROM project_files WHERE project_id = ? ORDER BY path",
  )
    .bind(project.id)
    .all<{ path: string; size: number; updated_at: string }>();

  return c.json(results.map((f) => ({ path: f.path, size: f.size, updatedAt: f.updated_at })));
});
