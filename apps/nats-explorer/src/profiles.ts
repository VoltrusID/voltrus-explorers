// Saved NATS connection profiles, persisted in localStorage.
//
// Secrets are NEVER written to localStorage: passwords, tokens, NKey seeds,
// JWTs and inline credentials stay out of `ConnProfile`.
// They live only in `sessionSecrets` (process memory) so switching between
// profiles in one run does not lose what you typed. After a restart you
// re-enter them. A `.creds` file is stored as a path, never its contents.

export type AuthMode = "none" | "userpass" | "token" | "creds_file" | "creds" | "nkey" | "jwt";

export interface ConnProfile {
  name: string;
  host: string;
  port: number;
  clientName: string;
  tls: boolean;
  authMode: AuthMode;
  /** Username is an identifier, not a secret. */
  username?: string;
  /** Path of a .creds file on disk (the file itself is not copied). */
  credsPath?: string;
}

/** Secret fields held in memory only, keyed by profile name. */
export interface ProfileSecrets {
  password: string;
  token: string;
  credsBody: string;
  nkeySeed: string;
  jwt: string;
  jwtSeed: string;
}

const KEY = "nats-explorer:profiles:v1";

const sessionSecrets = new Map<string, ProfileSecrets>();

export function getSessionSecrets(name: string): ProfileSecrets | undefined {
  return sessionSecrets.get(name);
}

export function setSessionSecrets(name: string, secrets: ProfileSecrets): void {
  sessionSecrets.set(name, secrets);
}

/** Whether connecting with this profile needs a secret the user must type again. */
export function profileNeedsSecret(p: ConnProfile): boolean {
  return p.authMode !== "none" && p.authMode !== "creds_file";
}

const AUTH_MODES: AuthMode[] = ["none", "userpass", "token", "creds_file", "creds", "nkey", "jwt"];

function isProfile(p: unknown): p is ConnProfile {
  if (!p || typeof p !== "object") return false;
  const o = p as Record<string, unknown>;
  return (
    typeof o.name === "string" &&
    typeof o.host === "string" &&
    typeof o.port === "number" &&
    typeof o.clientName === "string" &&
    typeof o.tls === "boolean" &&
    typeof o.authMode === "string" &&
    AUTH_MODES.includes(o.authMode as AuthMode)
  );
}

export function loadProfiles(): ConnProfile[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr.filter(isProfile);
  } catch {
    return [];
  }
}

function persist(list: ConnProfile[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    // Ignore quota / private-mode errors: profiles are a convenience.
  }
}

export function upsertProfile(p: ConnProfile): ConnProfile[] {
  const list = loadProfiles();
  const i = list.findIndex((x) => x.name === p.name);
  if (i >= 0) list[i] = p;
  else list.push(p);
  persist(list);
  return list;
}

export function deleteProfile(name: string): ConnProfile[] {
  sessionSecrets.delete(name);
  const list = loadProfiles().filter((x) => x.name !== name);
  persist(list);
  return list;
}

/** Saved profiles the free tier may keep; more needs Pro. */
export const FREE_PROFILE_LIMIT = 1;
