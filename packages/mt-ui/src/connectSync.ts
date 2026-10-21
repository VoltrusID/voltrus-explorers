// Pure bookkeeping for Voltrus Connect profile sync (no React, no Tauri): which local profiles changed,
// what a sync pass returned, and how that is applied to the app's own storage.

export type ProfileData = Record<string, unknown>;

/** A profile in the app's own storage. `key` is opaque and stable (Modbus uses the profile name). */
export interface LocalProfile {
  key: string;
  name: string;
  data: ProfileData;
}

/** Mirrors `mt_connect::Profile`. */
export interface ServerProfile {
  id: string;
  app: string;
  name: string;
  team_id: string | null;
  data: ProfileData;
  version: number;
  seq: number;
  deleted: boolean;
  can_edit: boolean;
  updated_at: number | null;
}

/** Mirrors `mt_connect::sync::Op`. */
export type SyncOp =
  | { op: "saved"; key: string; profile: ServerProfile }
  | { op: "pulled"; key: string; profile: ServerProfile }
  | { op: "added"; profile: ServerProfile }
  | { op: "removed"; key: string }
  | { op: "conflict"; key: string; server: ServerProfile | null }
  | { op: "deleted"; id: string }
  | { op: "failed"; key: string; message: string };

/** What the app has to offer so Connect can sync it. */
export interface ConnectStorage {
  list(): LocalProfile[];
  /** Creates (no `key`) or replaces a profile; returns its key. */
  upsert(profile: { key?: string; name: string; data: ProfileData }): string;
  remove(key: string): void;
}

export interface LinkMeta {
  /** Server profile id. */
  id: string;
  /** Server version this device last saw. */
  version: number;
  canEdit: boolean;
  /** Name when last synced, to describe a profile the user deleted locally. */
  name: string;
  /** `snapshotOf` the profile as last synced; a different snapshot means "changed here". */
  snapshot: string;
  teamId: string | null;
}

export interface SyncMeta {
  cursor: number | null;
  /** `entitlementSignature` at the last sync; a change forces a full list. */
  signature: string | null;
  lastSyncAt: number | null;
  links: Record<string, LinkMeta>;
  /** Team the user chose for a profile, ahead of its next upload. */
  teams: Record<string, string | null>;
}

export const emptyMeta = (): SyncMeta => ({ cursor: null, signature: null, lastSyncAt: null, links: {}, teams: {} });

/** JSON with sorted object keys, so the same value always serializes the same. */
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    const o = value as Record<string, unknown>;
    return `{${Object.keys(o)
      .sort()
      .filter((k) => o[k] !== undefined)
      .map((k) => `${JSON.stringify(k)}:${canonical(o[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

export const snapshotOf = (name: string, data: ProfileData, teamId: string | null) => canonical([name, data, teamId]);

/** The team a local profile is (or is about to be) shared with. */
export const teamFor = (meta: SyncMeta, key: string): string | null =>
  key in meta.teams ? (meta.teams[key] ?? null) : (meta.links[key]?.teamId ?? null);

export const isDirty = (meta: SyncMeta, p: LocalProfile): boolean => {
  const link = meta.links[p.key];
  return !link || link.snapshot !== snapshotOf(p.name, p.data, teamFor(meta, p.key));
};

export interface SyncRequest {
  locals: { key: string; name: string; data: ProfileData; team_id: string | null; link: { id: string; version: number } | null; dirty: boolean }[];
  deleted: string[];
  cursor: number | null;
  full: boolean;
}

/** Linked profiles that no longer exist locally: the user deleted them here. */
export function localDeletions(locals: LocalProfile[], meta: SyncMeta): { key: string; id: string; name: string }[] {
  const present = new Set(locals.map((l) => l.key));
  return Object.entries(meta.links)
    .filter(([key]) => !present.has(key))
    .map(([key, l]) => ({ key, id: l.id, name: l.name }));
}

/** Account state that changes which profiles are visible: re-list everything when it changes. */
export const entitlementSignature = (me: { entitlement: { active: boolean }; teams: { id: string }[] }) =>
  `${me.entitlement.active ? "1" : "0"}|${me.teams.map((t) => t.id).sort().join(",")}`;

export function buildRequest(locals: LocalProfile[], meta: SyncMeta, deleted: string[], full: boolean): SyncRequest {
  return {
    locals: locals.map((p) => {
      const link = meta.links[p.key];
      return {
        key: p.key,
        name: p.name,
        data: p.data,
        team_id: teamFor(meta, p.key),
        link: link ? { id: link.id, version: link.version } : null,
        dirty: isDirty(meta, p),
      };
    }),
    deleted,
    cursor: meta.cursor,
    full: full || meta.cursor === null,
  };
}

/** `name`, or `name (cloud)` / `name (cloud 2)` when taken. */
export function uniqueName(name: string, taken: Set<string>): string {
  if (!taken.has(name)) return name;
  let candidate = `${name} (cloud)`;
  for (let n = 2; taken.has(candidate); n++) candidate = `${name} (cloud ${n})`;
  return candidate;
}

export interface ApplyResult {
  meta: SyncMeta;
  /** Both sides changed: the user picks. `null` when the server would not let us read its copy. */
  conflicts: Record<string, ServerProfile | null>;
  failures: { key: string; message: string }[];
  counts: { uploaded: number; downloaded: number; removed: number };
}

/**
 * Applies a sync pass to local storage and returns the new bookkeeping. `sent` is what was handed to the
 * server, so a snapshot describes what was uploaded even if the user kept editing meanwhile.
 */
export function applyOps(
  ops: SyncOp[],
  sent: SyncRequest["locals"],
  storage: ConnectStorage,
  before: SyncMeta,
  cursor: number | null,
  signature: string,
  now: number,
): ApplyResult {
  const meta: SyncMeta = { ...before, links: { ...before.links }, teams: { ...before.teams }, cursor, signature, lastSyncAt: now };
  const sentByKey = new Map(sent.map((s) => [s.key, s]));
  const conflicts: ApplyResult["conflicts"] = {};
  const failures: ApplyResult["failures"] = [];
  const counts = { uploaded: 0, downloaded: 0, removed: 0 };

  const link = (key: string, p: ServerProfile, snapshot: string) => {
    meta.links[key] = { id: p.id, version: p.version, canEdit: p.can_edit, name: p.name, snapshot, teamId: p.team_id };
    delete meta.teams[key];
  };

  for (const op of ops) {
    switch (op.op) {
      case "saved": {
        const s = sentByKey.get(op.key);
        link(op.key, op.profile, snapshotOf(s?.name ?? op.profile.name, s?.data ?? op.profile.data, op.profile.team_id));
        counts.uploaded++;
        break;
      }
      case "pulled": {
        const key = storage.upsert({ key: op.key, name: op.profile.name, data: op.profile.data });
        if (key !== op.key) {
          delete meta.links[op.key];
          delete meta.teams[op.key];
        }
        link(key, op.profile, snapshotOf(op.profile.name, op.profile.data, op.profile.team_id));
        counts.downloaded++;
        break;
      }
      case "added": {
        const taken = new Set(storage.list().map((l) => l.name));
        const name = uniqueName(op.profile.name, taken);
        const key = storage.upsert({ name, data: op.profile.data });
        // A renamed copy differs from the server's name on purpose: the rename uploads on the next pass.
        link(key, op.profile, snapshotOf(op.profile.name, op.profile.data, op.profile.team_id));
        counts.downloaded++;
        break;
      }
      case "removed":
        storage.remove(op.key);
        delete meta.links[op.key];
        delete meta.teams[op.key];
        counts.removed++;
        break;
      case "conflict":
        conflicts[op.key] = op.server;
        break;
      case "deleted":
        for (const [k, l] of Object.entries(meta.links)) if (l.id === op.id) delete meta.links[k];
        break;
      case "failed":
        failures.push({ key: op.key, message: op.message });
        break;
    }
  }
  return { meta, conflicts, failures, counts };
}

/** Take the server's copy of a conflicted profile. */
export function acceptServerCopy(key: string, server: ServerProfile, storage: ConnectStorage, before: SyncMeta): SyncMeta {
  const newKey = storage.upsert({ key, name: server.name, data: server.data });
  const meta: SyncMeta = { ...before, links: { ...before.links }, teams: { ...before.teams } };
  if (newKey !== key) delete meta.links[key];
  delete meta.teams[key];
  meta.links[newKey] = {
    id: server.id,
    version: server.version,
    canEdit: server.can_edit,
    name: server.name,
    snapshot: snapshotOf(server.name, server.data, server.team_id),
    teamId: server.team_id,
  };
  return meta;
}
