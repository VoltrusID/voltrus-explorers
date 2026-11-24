import { Hono } from "hono";
import { authMiddleware } from "../auth";
import type { AppEnv } from "../env";
import { effectivePlan, FREE_MAX_FILES_PER_PROJECT } from "../entitlements";
import { getOwnedProject } from "./projects";

export const filesRoutes = new Hono<AppEnv>();

filesRoutes.use("*", authMiddleware);

// Paths are project-relative, stored flat in R2 as `{userId}/{projectId}/{path}`.
function isValidPath(path: string): boolean {
  return (
    path.length > 0 &&
    path.length <= 512 &&
    !path.includes("..") &&
    !path.includes("\0") &&
    !path.startsWith("/")
  );
}

function requestedPath(c: { req: { param: (name: string) => string | undefined } }): string {
  return c.req.param("path") ?? "";
}

filesRoutes.get("/:path{.+}", async (c) => {
  const path = requestedPath(c);
  if (!isValidPath(path)) return c.json({ error: "invalid path" }, 400);

  const project = await getOwnedProject(c.env.DB, c.get("userId"), c.req.param("id")!);
  if (!project) return c.json({ error: "project not found" }, 404);

  const object = await c.env.FILES.get(`${c.get("userId")}/${project.id}/${path}`);
  if (!object) return c.json({ error: "file not found" }, 404);

  return c.json({ content: await object.text() });
});

filesRoutes.put("/:path{.+}", async (c) => {
  const path = requestedPath(c);
  if (!isValidPath(path)) return c.json({ error: "invalid path" }, 400);

  const project = await getOwnedProject(c.env.DB, c.get("userId"), c.req.param("id")!);
  if (!project) return c.json({ error: "project not found" }, 404);

  const body = await c.req.json().catch(() => null);
  const content = body && typeof body === "object" ? (body as Record<string, unknown>).content : undefined;
  if (typeof content !== "string") return c.json({ error: "content is required" }, 400);

  // Free tier caps files per project. Overwriting an existing row never counts
  // against the quota — only new files do.
  if ((await effectivePlan(c.env.DB, c.get("userId"))) === "free") {
    const existing = await c.env.DB.prepare("SELECT 1 AS row FROM project_files WHERE project_id = ? AND path = ?")
      .bind(project.id, path)
      .first<{ row: number }>();
    if (!existing) {
      const count = await c.env.DB.prepare("SELECT COUNT(*) AS n FROM project_files WHERE project_id = ?")
        .bind(project.id)
        .first<{ n: number }>();
      if ((count?.n ?? 0) >= FREE_MAX_FILES_PER_PROJECT) {
        return c.json({ error: "plan_required", requiredPlan: "solo" }, 402);
      }
    }
  }

  const size = new TextEncoder().encode(content).byteLength;
  await c.env.FILES.put(`${c.get("userId")}/${project.id}/${path}`, content, {
    httpMetadata: { contentType: "text/plain; charset=utf-8" },
  });

  const now = new Date().toISOString();
  await c.env.DB.prepare(
    `INSERT INTO project_files (project_id, path, size, updated_at) VALUES (?, ?, ?, ?)
     ON CONFLICT(project_id, path) DO UPDATE SET size = excluded.size, updated_at = excluded.updated_at`,
  )
    .bind(project.id, path, size, now)
    .run();

  return c.body(null, 204);
});

filesRoutes.delete("/:path{.+}", async (c) => {
  const path = requestedPath(c);
  if (!isValidPath(path)) return c.json({ error: "invalid path" }, 400);

  const project = await getOwnedProject(c.env.DB, c.get("userId"), c.req.param("id")!);
  if (!project) return c.json({ error: "project not found" }, 404);

  await c.env.FILES.delete(`${c.get("userId")}/${project.id}/${path}`);
  await c.env.DB.prepare("DELETE FROM project_files WHERE project_id = ? AND path = ?").bind(project.id, path).run();

  return c.body(null, 204);
});
