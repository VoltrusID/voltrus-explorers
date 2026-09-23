import { Hono, type Context } from "hono";
import type { AppEnv, Env } from "./env";
import { authRoutes } from "./routes/auth";
import { projectsRoutes } from "./routes/projects";
import { filesRoutes } from "./routes/files";

function isAllowedOrigin(c: Context<AppEnv>): string | null {
  const origin = c.req.header("Origin");
  if (!origin) return null;
  const allowed: string[] = c.env.CORS_ORIGINS.split(",").map((s) => s.trim());
  return allowed.includes(origin) ? origin : null;
}

export function createApp() {
  const app = new Hono<AppEnv>();

  app.use("*", async (c, next) => {
    const origin = isAllowedOrigin(c);

    if (c.req.method === "OPTIONS") {
      const headers = new Headers();
      if (origin) {
        headers.set("Access-Control-Allow-Origin", origin);
        headers.set("Vary", "Origin");
        headers.set("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS");
        headers.set("Access-Control-Allow-Headers", "Authorization, Content-Type");
        headers.set("Access-Control-Max-Age", "86400");
      }
      return new Response(null, { status: 204, headers });
    }

    await next();

    if (origin) {
      c.res.headers.set("Access-Control-Allow-Origin", origin);
      c.res.headers.set("Vary", "Origin");
      c.res.headers.set("Access-Control-Allow-Headers", "Authorization, Content-Type");
    }
  });

  app.get("/api/health", (c) => c.json({ ok: true }));

  app.route("/api/auth", authRoutes);
  app.route("/api/projects", projectsRoutes);
  app.route("/api/projects/:id/files", filesRoutes);

  return app;
}

export type { AppEnv, Env };
