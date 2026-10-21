import { describe, it, expect } from "vitest";
import {
  acceptServerCopy,
  applyOps,
  buildRequest,
  canonical,
  emptyMeta,
  entitlementSignature,
  isDirty,
  localDeletions,
  snapshotOf,
  teamFor,
  uniqueName,
  type ConnectStorage,
  type LocalProfile,
  type ServerProfile,
  type SyncMeta,
} from "../../../../packages/mt-ui/src/connectSync";
import { fromData, toLocal, upsertProfile } from "../connectProfiles";
import type { ConnProfile } from "../types";

function memoryStorage(initial: LocalProfile[] = []) {
  let items = [...initial];
  const storage: ConnectStorage = {
    list: () => items,
    upsert: ({ key, name, data }) => {
      items = [...items.filter((p) => p.key !== key && p.key !== name), { key: name, name, data }];
      return name;
    },
    remove: (key) => {
      items = items.filter((p) => p.key !== key);
    },
  };
  return { storage, items: () => items };
}

const server = (over: Partial<ServerProfile> = {}): ServerProfile => ({
  id: "id-1",
  app: "modbus-poll",
  name: "Pump",
  team_id: null,
  data: { n: 1 },
  version: 1,
  seq: 1,
  deleted: false,
  can_edit: true,
  updated_at: 0,
  ...over,
});

const linked = (key: string, snapshot: string, over: Partial<SyncMeta["links"][string]> = {}): SyncMeta => ({
  ...emptyMeta(),
  links: { [key]: { id: "id-1", version: 1, canEdit: true, name: key, snapshot, teamId: null, ...over } },
});

describe("change tracking", () => {
  it("serializes objects the same regardless of key order", () => {
    expect(canonical({ b: 1, a: { d: [1, 2], c: null } })).toBe(canonical({ a: { c: null, d: [1, 2] }, b: 1 }));
  });

  it("treats unlinked profiles as changed and linked ones by snapshot", () => {
    const p: LocalProfile = { key: "Pump", name: "Pump", data: { n: 1 } };
    expect(isDirty(emptyMeta(), p)).toBe(true);
    const meta = linked("Pump", snapshotOf("Pump", { n: 1 }, null));
    expect(isDirty(meta, p)).toBe(false);
    expect(isDirty(meta, { ...p, data: { n: 2 } })).toBe(true);
  });

  it("counts a chosen team as a change until it is uploaded", () => {
    const p: LocalProfile = { key: "Pump", name: "Pump", data: { n: 1 } };
    const meta = { ...linked("Pump", snapshotOf("Pump", { n: 1 }, null)), teams: { Pump: "team-1" } };
    expect(teamFor(meta, "Pump")).toBe("team-1");
    expect(isDirty(meta, p)).toBe(true);
  });

  it("finds profiles deleted on this device", () => {
    const meta = linked("Gone", "x");
    expect(localDeletions([], meta)).toEqual([{ key: "Gone", id: "id-1", name: "Gone" }]);
    expect(localDeletions([{ key: "Gone", name: "Gone", data: {} }], meta)).toEqual([]);
  });

  it("forces a full list on first sync and when teams or entitlement change", () => {
    expect(buildRequest([], emptyMeta(), [], false).full).toBe(true);
    expect(buildRequest([], { ...emptyMeta(), cursor: 5 }, [], false).full).toBe(false);
    expect(entitlementSignature({ entitlement: { active: true }, teams: [{ id: "b" }, { id: "a" }] })).toBe("1|a,b");
  });

  it("names a cloud copy that collides with a local profile", () => {
    expect(uniqueName("Pump", new Set())).toBe("Pump");
    expect(uniqueName("Pump", new Set(["Pump"]))).toBe("Pump (cloud)");
    expect(uniqueName("Pump", new Set(["Pump", "Pump (cloud)"]))).toBe("Pump (cloud 2)");
  });
});

describe("applying a sync pass", () => {
  it("links an uploaded profile to what was sent, not to later edits", () => {
    const { storage } = memoryStorage([{ key: "Pump", name: "Pump", data: { n: 2 } }]);
    const sent = buildRequest(storage.list(), emptyMeta(), [], true).locals;
    const res = applyOps([{ op: "saved", key: "Pump", profile: server({ data: { n: 2 }, version: 1 }) }], sent, storage, emptyMeta(), 9, "0|", 1000);
    expect(res.meta.links.Pump).toMatchObject({ id: "id-1", version: 1 });
    expect(isDirty(res.meta, { key: "Pump", name: "Pump", data: { n: 2 } })).toBe(false);
    expect(res.meta.cursor).toBe(9);
    expect(res.counts.uploaded).toBe(1);
  });

  it("downloads new profiles, renaming on a name clash and uploading the rename later", () => {
    const { storage, items } = memoryStorage([{ key: "Pump", name: "Pump", data: { mine: true } }]);
    const res = applyOps([{ op: "added", profile: server({ id: "id-2", name: "Pump", data: { theirs: true } }) }], [], storage, emptyMeta(), 1, "s", 0);
    expect(items().map((p) => p.name)).toEqual(["Pump", "Pump (cloud)"]);
    expect(isDirty(res.meta, { key: "Pump (cloud)", name: "Pump (cloud)", data: { theirs: true } })).toBe(true);
    expect(res.meta.links["Pump (cloud)"].id).toBe("id-2");
  });

  it("replaces a clean profile with a newer server copy", () => {
    const { storage, items } = memoryStorage([{ key: "Pump", name: "Pump", data: { n: 1 } }]);
    const res = applyOps(
      [{ op: "pulled", key: "Pump", profile: server({ name: "Pump", data: { n: 2 }, version: 2 }) }],
      [],
      storage,
      linked("Pump", snapshotOf("Pump", { n: 1 }, null)),
      1,
      "s",
      0,
    );
    expect(items()[0].data).toEqual({ n: 2 });
    expect(res.meta.links.Pump.version).toBe(2);
    expect(isDirty(res.meta, items()[0])).toBe(false);
  });

  it("follows a rename made on another device", () => {
    const { storage, items } = memoryStorage([{ key: "Pump", name: "Pump", data: { n: 1 } }]);
    const res = applyOps(
      [{ op: "pulled", key: "Pump", profile: server({ name: "Pump house", version: 2 }) }],
      [],
      storage,
      linked("Pump", snapshotOf("Pump", { n: 1 }, null)),
      1,
      "s",
      0,
    );
    expect(items().map((p) => p.key)).toEqual(["Pump house"]);
    expect(Object.keys(res.meta.links)).toEqual(["Pump house"]);
  });

  it("removes profiles deleted elsewhere and forgets deletions the server confirmed", () => {
    const { storage, items } = memoryStorage([{ key: "Pump", name: "Pump", data: {} }]);
    const meta = linked("Pump", "s");
    const removed = applyOps([{ op: "removed", key: "Pump" }], [], storage, meta, 1, "s", 0);
    expect(items()).toEqual([]);
    expect(removed.meta.links).toEqual({});
    const deleted = applyOps([{ op: "deleted", id: "id-1" }], [], memoryStorage().storage, meta, 1, "s", 0);
    expect(deleted.meta.links).toEqual({});
  });

  it("reports conflicts and failures without touching local data", () => {
    const { storage, items } = memoryStorage([{ key: "Pump", name: "Pump", data: { mine: 1 } }]);
    const meta = linked("Pump", "old");
    const res = applyOps(
      [
        { op: "conflict", key: "Pump", server: server({ version: 5 }) },
        { op: "failed", key: "Team", message: "Connect subscription required" },
      ],
      [],
      storage,
      meta,
      1,
      "s",
      0,
    );
    expect(res.conflicts.Pump?.version).toBe(5);
    expect(res.failures).toEqual([{ key: "Team", message: "Connect subscription required" }]);
    expect(items()[0].data).toEqual({ mine: 1 });
    expect(res.meta.links.Pump.version).toBe(1);
  });

  it("taking the server's copy of a conflict clears the local change", () => {
    const { storage, items } = memoryStorage([{ key: "Pump", name: "Pump", data: { mine: 1 } }]);
    const meta = acceptServerCopy("Pump", server({ data: { theirs: 1 }, version: 6 }), storage, linked("Pump", "old"));
    expect(items()[0].data).toEqual({ theirs: 1 });
    expect(meta.links.Pump.version).toBe(6);
    expect(isDirty(meta, items()[0])).toBe(false);
  });
});

describe("Modbus profile adapter", () => {
  const profile: ConnProfile = {
    name: "Boiler",
    transport: "tcp",
    tcp: { host: "10.0.0.5", port: 502, unitId: 3, pollInterval: 500 },
    rtu: { port: "", baudRate: 9600, dataBits: 8, stopBits: 1, parity: "none", unitId: 1 },
    blocks: [],
  };

  it("round-trips a saved connection through the profile data", () => {
    const local = toLocal(profile);
    expect(local.key).toBe("Boiler");
    expect(fromData("Boiler", local.data)).toEqual(profile);
  });

  it("fills in what a profile from another device is missing", () => {
    const p = fromData("X", { transport: "weird", tcp: { host: "h" }, blocks: "nope" });
    expect(p.transport).toBe("tcp");
    expect(p.tcp).toEqual({ host: "h", port: 502, unitId: 1, pollInterval: 1000 });
    expect(p.rtu.baudRate).toBe(9600);
    expect(p.blocks).toEqual([]);
  });

  it("replaces in place, adds at the end, and drops a name clash", () => {
    const a = { ...profile, name: "A" };
    const b = { ...profile, name: "B" };
    const c = { ...profile, name: "C" };
    expect(upsertProfile([a, b, c], "B", { ...b, tcp: { ...b.tcp, port: 1502 } }).map((p) => p.name)).toEqual(["A", "B", "C"]);
    expect(upsertProfile([a, b], undefined, c).map((p) => p.name)).toEqual(["A", "B", "C"]);
    expect(upsertProfile([a, b, c], "A", { ...a, name: "C" }).map((p) => p.name)).toEqual(["C", "B"]);
  });
});
