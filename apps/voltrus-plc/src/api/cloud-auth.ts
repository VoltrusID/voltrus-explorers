// Thin REST client for the Voltrus PLC cloud worker (apps/voltrus-plc-cloud).
// Bridge commands (save_file / list_project_files / open_file) go through
// src/api/bridge.ts; account- and project-shaped REST lives here. The base URL
// mirrors the bridge: the vplc_base localStorage key, falling back to the
// default below.

// Same-origin API in production (one worker serves SPA + API); localhost dev
// targets the local wrangler dev server. vplc_base overrides.
const DEFAULT_BASE = /^localhost$|^127\.0\.0\.1$/.test(location.hostname)
  ? "http://localhost:8787"
  : "";

export function cloudApiBase(): string {
  return localStorage.getItem("vplc_base") ?? DEFAULT_BASE;
}

export interface CloudUser {
  id: string;
  email: string;
}

export interface CloudProject {
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  fileCount: number;
}

export class CloudApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
    this.name = "CloudApiError";
  }
}

async function request<T>(path: string, init: RequestInit, token?: string): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${cloudApiBase()}${path}`, {
      ...init,
      headers: {
        ...(init.body ? { "Content-Type": "application/json" } : {}),
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
    });
  } catch (e) {
    throw new CloudApiError(0, e instanceof Error ? e.message : String(e));
  }
  if (!res.ok) {
    let message = `HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { error?: unknown };
      if (typeof body.error === "string") message = body.error;
    } catch {
      // non-JSON error body — keep the HTTP status fallback
    }
    throw new CloudApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export interface AuthResponse {
  token: string;
  user: CloudUser;
}

export function register(email: string, password: string): Promise<AuthResponse> {
  return request("/api/auth/register", { method: "POST", body: JSON.stringify({ email, password }) });
}

export function login(email: string, password: string): Promise<AuthResponse> {
  return request("/api/auth/login", { method: "POST", body: JSON.stringify({ email, password }) });
}

export function whoami(token: string): Promise<{ user: CloudUser }> {
  return request("/api/auth/me", {}, token);
}

export function listProjects(token: string): Promise<CloudProject[]> {
  return request("/api/projects", {}, token);
}

export function createProject(
  token: string,
  name: string,
): Promise<Omit<CloudProject, "fileCount">> {
  return request("/api/projects", { method: "POST", body: JSON.stringify({ name }) }, token);
}

export async function deleteProject(token: string, id: string): Promise<void> {
  await request<void>(`/api/projects/${encodeURIComponent(id)}`, { method: "DELETE" }, token);
}
