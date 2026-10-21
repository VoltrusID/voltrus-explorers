import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Button } from "@mactools/ui";
import { STANDARD_CLASSES, classMeta, type CipClassMeta } from "./cipMeta";
import { edsAttrName, type EdsMap } from "./eds";

const toHex = (b: number[]) =>
  b.map((x) => x.toString(16).padStart(2, "0")).join(" ");

interface InstEntry {
  instance: number;
  data: number[];
}

interface Props {
  eds: EdsMap;
  onLog?: (direction: "tx" | "rx" | "error" | "info", message: string) => void;
}

/**
 * Auto-expanding CIP object tree. Renders the standard classes with friendly
 * attribute names; reading values uses Get Attribute Single (0x0E) and instance
 * enumeration uses Get Attributes All (0x01) via the existing backend commands.
 */
export function CipTree({ eds, onLog }: Props) {
  const [expanded, setExpanded] = useState<Set<number>>(new Set([0x01]));
  const [instances, setInstances] = useState<Record<number, InstEntry[]>>({});
  const [values, setValues] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busyCls, setBusyCls] = useState<Set<number>>(new Set());

  const toggle = (id: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const readAttr = useCallback(
    async (cls: number, inst: number, attr: number) => {
      const key = `${cls}:${inst}:${attr}`;
      try {
        const bytes = await invoke<number[]>("cip_get_attribute", {
          classId: cls,
          instanceId: inst,
          attributeId: attr,
        });
        const meta = classMeta(cls)?.attrs.find((a) => a.id === attr);
        const decoded = meta?.decode ? meta.decode(bytes) : bytes.length ? toHex(bytes) : "(empty)";
        setValues((v) => ({ ...v, [key]: decoded }));
        onLog?.("rx", `Get ${cls.toString(16)}/${inst}/${attr} → ${bytes.length}B`);
      } catch (e) {
        setErrors((er) => ({ ...er, [key]: String(e) }));
        onLog?.("error", `Get ${cls.toString(16)}/${inst}/${attr} failed: ${String(e)}`);
      }
    },
    [onLog],
  );

  const readAllAttrs = useCallback(
    async (meta: CipClassMeta, inst: number) => {
      await Promise.all(meta.attrs.map((a) => readAttr(meta.id, inst, a.id)));
    },
    [readAttr],
  );

  const listInstances = useCallback(
    async (cls: number) => {
      setBusyCls((b) => new Set(b).add(cls));
      try {
        const list = await invoke<InstEntry[]>("list_class_instances", {
          classId: cls,
          max: 256,
        });
        setInstances((prev) => ({ ...prev, [cls]: list }));
        onLog?.("rx", `Class 0x${cls.toString(16)} → ${list.length} instance(s)`);
      } catch (e) {
        onLog?.("error", `List instances 0x${cls.toString(16)} failed: ${String(e)}`);
      } finally {
        setBusyCls((b) => {
          const n = new Set(b);
          n.delete(cls);
          return n;
        });
      }
    },
    [onLog],
  );

  // Auto-read known attributes + instance list when a class is first expanded.
  const onExpand = (meta: CipClassMeta) => {
    toggle(meta.id);
    if (!expanded.has(meta.id)) {
      if (meta.attrs.length) readAllAttrs(meta, 1);
      if (instances[meta.id] === undefined) listInstances(meta.id);
    }
  };

  return (
    <div style={{ fontSize: 12, fontFamily: '"SF Mono", "JetBrains Mono", monospace' }}>
      {STANDARD_CLASSES.map((meta) => {
        const isOpen = expanded.has(meta.id);
        const insts = instances[meta.id] ?? [];
        return (
          <div key={meta.id} style={{ marginBottom: 2 }}>
            <div
              onClick={() => (meta.attrs.length ? onExpand(meta) : onExpand(meta))}
              style={{
                cursor: "pointer",
                padding: "3px 4px",
                borderRadius: 4,
                color: "var(--text)",
                display: "flex",
                alignItems: "center",
                gap: 6,
              }}
              onMouseEnter={(e) => ((e.currentTarget as HTMLElement).style.background = "var(--hover-overlay)")}
              onMouseLeave={(e) => ((e.currentTarget as HTMLElement).style.background = "transparent")}
            >
              <span style={{ color: "var(--accent)", width: 12, display: "inline-flex" }}>
                {isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
              </span>
              <span>{meta.name}</span>
              {insts.length > 0 && (
                <span style={{ color: "var(--text-muted)", fontSize: 10 }}>({insts.length} inst)</span>
              )}
            </div>

            {isOpen && (
              <div style={{ marginLeft: 16, marginTop: 2 }}>
                {meta.attrs.length > 0 && (
                  <div style={{ marginBottom: 6 }}>
                    <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 2 }}>
                      Instance 1: attributes
                    </div>
                    {meta.attrs.map((a) => {
                      const key = `${meta.id}:1:${a.id}`;
                      const edsName = edsAttrName(eds, meta.id, 1, a.id);
                      return (
                        <div
                          key={a.id}
                          style={{ display: "flex", gap: 8, padding: "2px 4px", borderBottom: "1px solid var(--border)" }}
                        >
                          <span style={{ color: "var(--text-dim)", width: 28 }}>{a.id}</span>
                          <span style={{ color: "var(--text)", minWidth: 140, flex: 1 }}>
                            {edsName ?? a.name}
                            {edsName && edsName !== a.name && (
                              <span style={{ color: "var(--text-muted)" }}> ({a.name})</span>
                            )}
                          </span>
                          <span style={{ color: "var(--text-muted)", fontSize: 10, width: 60 }}>{a.type}</span>
                          <span style={{ color: "var(--accent)", flex: 2 }}>
                            {values[key] ?? (errors[key] ? errors[key] : "—")}
                          </span>
                          <Button variant="ghost" size="sm" onClick={() => readAttr(meta.id, 1, a.id)}>
                            Read
                          </Button>
                        </div>
                      );
                    })}
                  </div>
                )}

                <div style={{ marginBottom: 4 }}>
                  <Button variant="ghost" size="sm" disabled={busyCls.has(meta.id)} onClick={() => listInstances(meta.id)}>
                    {busyCls.has(meta.id) ? "Listing" : "List instances"}
                  </Button>
                </div>
                {insts.length > 0 && (
                  <div style={{ maxHeight: 120, overflow: "auto" }}>
                    {insts.map((it) => (
                      <div key={it.instance} style={{ display: "flex", gap: 8, padding: "1px 4px", fontSize: 11 }}>
                        <span style={{ color: "var(--text-dim)", width: 70 }}>inst {it.instance}</span>
                        <span style={{ color: "var(--text-muted)" }}>{it.data.length}B</span>
                        <span style={{ color: "var(--text-dim)", flex: 1 }}>{toHex(it.data.slice(0, 16))}{it.data.length > 16 ? "…" : ""}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
