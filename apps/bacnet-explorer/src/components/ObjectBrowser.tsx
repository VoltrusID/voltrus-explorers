import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  Bell,
  Box,
  Calendar,
  CalendarClock,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Download,
  Eye,
  Gauge,
  LineChart,
  ListTree,
  Server,
  Star,
  ToggleLeft,
  type LucideIcon,
} from "lucide-react";
import { Button, EmptyState, IconButton, ProBadge, saveTextFile, useProStatus } from "@mactools/ui";
import type { BacnetObjectDetail, BacnetObjectInfo, BacnetPropertyEntry, BacnetValue } from "../types";
import { formatObjectType } from "../types";
import { buildObjectListCsv, type ObjectListRow } from "../objectCsv";
import { trendKey } from "../hooks/useBacnetTrend";
import { PropertyInspector } from "./PropertyInspector";
import { DeviceInspector } from "./DeviceInspector";

/** Map a BACnet object type to a representative icon. */
function objectIconFor(ot: string): LucideIcon {
  if (ot.startsWith("analogInput") || ot.startsWith("analogOutput")) return Gauge;
  if (ot.startsWith("analog")) return Activity;
  if (ot.startsWith("binary")) return ToggleLeft;
  if (ot.startsWith("multiState")) return ListTree;
  if (ot === "device") return Server;
  if (ot.startsWith("trendLog")) return LineChart;
  if (ot === "schedule") return CalendarClock;
  if (ot === "calendar") return Calendar;
  if (ot === "notificationClass") return Bell;
  return Box;
}

function ObjectTypeIcon({ type, size = 14 }: { type: string; size?: number }) {
  const Icon = objectIconFor(type);
  return <Icon size={size} style={{ flexShrink: 0 }} />;
}

interface ObjectBrowserProps {
  onObjectSelect?: (obj: BacnetObjectInfo) => void;
  trendKeys?: Set<string>;
  onToggleTrend?: (obj: { type: string; instance: number }, name: string) => void;
  /** Open the trend log viewer for a trendLog / trendLogMultiple object. */
  onOpenTrendLog?: (obj: { type: "trendLog" | "trendLogMultiple"; instance: number }, name: string) => void;
  /** Open the schedule viewer for a schedule object. */
  onOpenSchedule?: (instance: number, name: string) => void;
  /**
   * "split" shows the object tree and the inspector side by side (desktop, iPad).
   * "stacked" shows one at a time: the tree, then the selected object's properties with a back button (phone).
   */
  layout?: "split" | "stacked";
  /** Width of the tree pane in the split layout. */
  treeWidth?: number;
  /** False while another open device is shown: this browser stays mounted but must not read. */
  active?: boolean;
  /** Used in the exported file name. */
  deviceLabel?: string;
  watchKeys?: Set<string>;
  onToggleWatch?: (obj: { type: string; instance: number }, name: string) => void;
  /** One-line result for the communication log after an export finished or failed. */
  onExportResult?: (level: "info" | "error", message: string) => void;
}

// ── Styles ──

const container: React.CSSProperties = {
  display: "flex",
  height: "100%",
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  overflow: "hidden",
};

const treePanel: React.CSSProperties = {
  width: 200,
  flexShrink: 0,
  borderRight: "1px solid var(--border)",
  display: "flex",
  flexDirection: "column",
  overflow: "hidden",
};

const treeHeader: React.CSSProperties = {
  padding: "10px 12px",
  borderBottom: "1px solid var(--border)",
  display: "flex",
  alignItems: "center",
  gap: 8,
};

const treeScroll: React.CSSProperties = {
  flex: 1,
  overflowY: "auto",
  padding: "4px 0",
};

const groupHeader: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  padding: "5px 12px",
  cursor: "pointer",
  fontSize: "var(--fs-xs)",
  fontWeight: 600,
  color: "var(--text-muted)",
  userSelect: "none",
  gap: 6,
};

const groupCount: React.CSSProperties = {
  marginLeft: "auto",
  fontSize: "var(--fs-xs)",
  color: "var(--text-dim)",
  fontFamily: "var(--font-mono)",
  fontVariantNumeric: "tabular-nums",
};

const objectRow: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  padding: "2px 6px 2px 28px",
  cursor: "pointer",
  fontSize: "var(--fs-sm)",
  color: "var(--text)",
  gap: 6,
};

const objectRowSelected: React.CSSProperties = {
  ...objectRow,
  background: "var(--selection)",
};

const instanceLabel: React.CSSProperties = {
  marginLeft: "auto",
  fontSize: "var(--fs-xs)",
  color: "var(--text-dim)",
  fontFamily: "var(--font-mono)",
  fontVariantNumeric: "tabular-nums",
};

const inspectorPanel: React.CSSProperties = {
  flex: 1,
  overflow: "auto",
  padding: 12,
};

// ── Helpers ──

function groupObjectsByType(objects: BacnetObjectInfo[]): Map<string, BacnetObjectInfo[]> {
  const groups = new Map<string, BacnetObjectInfo[]>();
  for (const obj of objects) {
    const list = groups.get(obj.object_type) ?? [];
    list.push(obj);
    groups.set(obj.object_type, list);
  }
  return groups;
}

function objectKey(obj: BacnetObjectInfo): string {
  return `${obj.object_type}:${obj.object_instance}`;
}

// ── Component ──

export function ObjectBrowser(props: ObjectBrowserProps) {
  const { onObjectSelect, trendKeys, onToggleTrend, onOpenTrendLog, onOpenSchedule, layout = "split", treeWidth } = props;
  const { active: deviceActive = true, deviceLabel, watchKeys, onToggleWatch, onExportResult } = props;
  const { requirePro } = useProStatus();
  const stacked = layout === "stacked";

  const [objects, setObjects] = useState<BacnetObjectInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedObject, setSelectedObject] = useState<{ type: string; instance: number } | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [properties, setProperties] = useState<BacnetPropertyEntry[]>([]);
  const [detail, setDetail] = useState<BacnetObjectDetail | null>(null);
  const [propertiesLoading, setPropertiesLoading] = useState(false);
  const [propertiesError, setPropertiesError] = useState<string | null>(null);
  const propertiesRequest = useRef(0);
  const [exportProgress, setExportProgress] = useState<{ done: number; total: number } | null>(null);
  const exportCancelled = useRef(false);
  const deviceActiveRef = useRef(deviceActive);
  deviceActiveRef.current = deviceActive;

  // One read per object (the browse reply carries no names), a few at a time.
  const exportList = useCallback(async () => {
    if (!requirePro("Export object list to CSV")) return;
    const list = objects;
    if (list.length === 0 || exportProgress) return;
    exportCancelled.current = false;
    setExportProgress({ done: 0, total: list.length });
    const rows: ObjectListRow[] = new Array(list.length);
    let next = 0;
    let done = 0;
    let stoppedBy: string | null = null;
    const worker = async () => {
      while (stoppedBy === null) {
        if (exportCancelled.current) {
          stoppedBy = "cancelled";
          return;
        }
        if (!deviceActiveRef.current) {
          stoppedBy = "another device was selected";
          return;
        }
        const index = next++;
        if (index >= list.length) return;
        const object = list[index];
        try {
          const detail = await invoke<BacnetObjectDetail | null>("read_object_properties", {
            objectType: object.object_type,
            objectInstance: object.object_instance,
          });
          rows[index] = { object, detail };
        } catch (e) {
          rows[index] = { object, detail: null, error: String(e) };
        }
        done += 1;
        setExportProgress({ done, total: list.length });
      }
    };
    try {
      await Promise.all(Array.from({ length: Math.min(4, list.length) }, worker));
      if (stoppedBy !== null) {
        if (stoppedBy !== "cancelled") onExportResult?.("error", `Object list export stopped: ${stoppedBy}`);
        return;
      }
      const failed = rows.filter((r) => r.error).length;
      const name = `bacnet-objects-${(deviceLabel ?? "device").replace(/[^A-Za-z0-9_.-]+/g, "-")}.csv`;
      const path = await saveTextFile(name, buildObjectListCsv(rows));
      if (path) {
        onExportResult?.(
          "info",
          `Object list exported to ${path} (${list.length} objects${failed ? `, ${failed} could not be read` : ""})`,
        );
      }
    } catch (e) {
      onExportResult?.("error", `Object list export failed: ${e}`);
    } finally {
      setExportProgress(null);
    }
  }, [objects, exportProgress, requirePro, deviceLabel, onExportResult]);

  // Leaving the view (unmount) stops a running export.
  useEffect(() => () => { exportCancelled.current = true; }, []);

  // Browse objects
  const handleBrowse = useCallback(async () => {
    setLoading(true);
    setError(null);
    setObjects([]);
    setSelectedObject(null);
    setProperties([]);
    setDetail(null);
    propertiesRequest.current += 1; // drop any in-flight property read
    exportCancelled.current = true; // an export in progress describes the old list
    setPropertiesError(null);
    setPropertiesLoading(false);
    try {
      const result = await invoke<BacnetObjectInfo[]>("browse_objects");
      setObjects(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  // Back from the properties page to the object list (stacked layout).
  const handleBack = useCallback(() => {
    propertiesRequest.current += 1; // drop any in-flight property read
    setSelectedObject(null);
    setProperties([]);
    setDetail(null);
    setPropertiesError(null);
    setPropertiesLoading(false);
  }, []);

  // Toggle group expand/collapse
  const toggleGroup = useCallback((objectType: string) => {
    setExpandedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(objectType)) {
        next.delete(objectType);
      } else {
        next.add(objectType);
      }
      return next;
    });
  }, []);

  // Read the selected object's properties. Responses for a superseded request
  // (user clicked another object, or re-browsed) are dropped.
  const loadProperties = useCallback(async (target: { type: string; instance: number }) => {
    const requestId = ++propertiesRequest.current;
    setPropertiesLoading(true);
    setPropertiesError(null);
    try {
      const result = await invoke<BacnetObjectDetail | null>("read_object_properties", {
        objectType: target.type,
        objectInstance: target.instance,
      });
      if (requestId !== propertiesRequest.current) return;
      setDetail(result);
      setProperties(result?.properties ?? []);
    } catch (e) {
      if (requestId !== propertiesRequest.current) return;
      setDetail(null);
      setProperties([]);
      setPropertiesError(`Reading ${formatObjectType(target.type)} ${target.instance} failed: ${String(e)}`);
    } finally {
      if (requestId === propertiesRequest.current) setPropertiesLoading(false);
    }
  }, []);

  // Select an object and load its properties
  const handleSelectObject = useCallback(
    async (obj: BacnetObjectInfo) => {
      setSelectedObject({ type: obj.object_type, instance: obj.object_instance });
      setProperties([]);
      setDetail(null);
      await loadProperties({ type: obj.object_type, instance: obj.object_instance });
      onObjectSelect?.(obj);
    },
    [onObjectSelect, loadProperties],
  );

  // Write a typed property value. Rejects with the device's error so the
  // inspector can show it; on success the properties are re-read.
  const handleWrite = useCallback(
    async (propertyName: string, value: BacnetValue) => {
      if (!selectedObject) throw new Error("No object selected");
      await invoke("write_tag", {
        tag: {
          name: propertyName,
          object_type: selectedObject.type,
          object_instance: selectedObject.instance,
          property: propertyName,
        },
        value,
      });
      await loadProperties(selectedObject);
    },
    [selectedObject, loadProperties],
  );

  // Render
  const groups = groupObjectsByType(objects);
  const isDeviceSelected = selectedObject?.type === "device";
  const detailOpen = stacked && selectedObject !== null;
  const selectedName = selectedObject
    ? objects.find((o) => o.object_type === selectedObject.type && o.object_instance === selectedObject.instance)?.object_name ||
      `${formatObjectType(selectedObject.type)} ${selectedObject.instance}`
    : "";

  return (
    <div style={stacked ? { ...container, flexDirection: "column", border: "none", borderRadius: 0, background: "transparent" } : container}>
      {/* Left panel — object tree. Hidden (not unmounted) behind the properties page so scroll position survives. */}
      <div
        style={
          stacked
            ? { ...treePanel, width: "100%", flex: 1, minHeight: 0, borderRight: "none", display: detailOpen ? "none" : "flex" }
            : treeWidth
              ? { ...treePanel, width: treeWidth }
              : treePanel
        }
      >
        {/* The phone page already has the "Objects" title. */}
        {!stacked && (
          <div style={treeHeader}>
            <span className="vt-panel__title">Objects</span>
          </div>
        )}

        <div style={{ padding: stacked ? "0 0 8px" : "8px 10px 4px" }}>
          <Button variant="primary" block icon={<ListTree size={14} strokeWidth={1.75} />} loading={loading} onClick={handleBrowse}>
            {loading ? "Browsing…" : "Browse objects"}
          </Button>
        </div>

        <div style={{ padding: stacked ? "0 0 8px" : "0 10px 4px" }}>
          {exportProgress ? (
            <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: 6, alignItems: "center" }}>
              <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontVariantNumeric: "tabular-nums" }}>
                Reading {exportProgress.done} of {exportProgress.total}
              </span>
              <Button size="sm" onClick={() => { exportCancelled.current = true; }}>
                Cancel
              </Button>
            </div>
          ) : (
            <Button block disabled={objects.length === 0} icon={<Download size={14} strokeWidth={1.75} />} onClick={exportList}>
              Export CSV <ProBadge />
            </Button>
          )}
        </div>

        {error && (
          <div style={{ margin: "4px 10px 8px", padding: "6px 8px", fontSize: "var(--fs-xs)", color: "var(--danger)", background: "var(--danger-soft)", borderRadius: "var(--radius)" }}>
            {error}
          </div>
        )}

        <div style={stacked ? { ...treeScroll, background: "var(--card)", borderRadius: "var(--radius-lg)" } : treeScroll}>
          {objects.length === 0 && !loading && !error && (
            <EmptyState
              compact
              title="No objects listed"
              description="Browse the connected device to list its objects."
            />
          )}

          {Array.from(groups.entries()).map(([objectType, items]) => {
            const expanded = expandedGroups.has(objectType);

            return (
              <div key={objectType}>
                {/* Group header */}
                <div className="ob-group" style={groupHeader} onClick={() => toggleGroup(objectType)}>
                  <span style={{ display: "inline-flex", width: 14, color: "var(--text-dim)" }}>
                    {expanded ? <ChevronDown size={14} strokeWidth={1.75} /> : <ChevronRight size={14} strokeWidth={1.75} />}
                  </span>
                  <ObjectTypeIcon type={objectType} />
                  <span>{formatObjectType(objectType)}</span>
                  <span style={groupCount}>{items.length}</span>
                </div>

                {/* Group children */}
                {expanded && (
                  <div>
                    {items
                      .sort((a, b) => a.object_instance - b.object_instance)
                      .map((obj) => {
                        const selected =
                          selectedObject?.type === obj.object_type &&
                          selectedObject?.instance === obj.object_instance;
                        const logType =
                          obj.object_type === "trendLog" || obj.object_type === "trendLogMultiple"
                            ? obj.object_type
                            : null;
                        return (
                          <div
                            key={objectKey(obj)}
                            className="ob-row"
                            style={selected ? objectRowSelected : objectRow}
                            onClick={() => handleSelectObject(obj)}
                          >
                            <ObjectTypeIcon type={obj.object_type} size={12} />
                            <span style={stacked ? { fontSize: "var(--fs-md)", minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" } : { fontSize: "var(--fs-sm)" }}>
                              {obj.object_name || `${formatObjectType(obj.object_type)} ${obj.object_instance}`}
                            </span>
                            <span style={instanceLabel}>{obj.object_instance}</span>
                            {onToggleWatch && (() => {
                              const k = trendKey(obj.object_type, obj.object_instance);
                              const watched = watchKeys?.has(k);
                              return (
                                <IconButton
                                  size="sm"
                                  label={watched ? "Remove from watch list" : "Add to watch list"}
                                  pressed={watched}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    onToggleWatch(
                                      { type: obj.object_type, instance: obj.object_instance },
                                      obj.object_name || `${formatObjectType(obj.object_type)} ${obj.object_instance}`,
                                    );
                                  }}
                                >
                                  <Star size={12} strokeWidth={1.75} />
                                </IconButton>
                              );
                            })()}
                            {onToggleTrend && (() => {
                              const k = trendKey(obj.object_type, obj.object_instance);
                              const active = trendKeys?.has(k);
                              return (
                                <IconButton
                                  size="sm"
                                  label={active ? "Remove from trend" : "Add to trend"}
                                  pressed={active}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    onToggleTrend(
                                      { type: obj.object_type, instance: obj.object_instance },
                                      obj.object_name || `${formatObjectType(obj.object_type)} ${obj.object_instance}`,
                                    );
                                  }}
                                >
                                  <LineChart size={12} strokeWidth={1.75} />
                                </IconButton>
                              );
                            })()}
                            {logType && onOpenTrendLog && (
                              <IconButton
                                size="sm"
                                label="View trend log"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  onOpenTrendLog(
                                    { type: logType, instance: obj.object_instance },
                                    obj.object_name || `${formatObjectType(obj.object_type)} ${obj.object_instance}`,
                                  );
                                }}
                              >
                                <Eye size={12} strokeWidth={1.75} />
                              </IconButton>
                            )}
                            {obj.object_type === "schedule" && onOpenSchedule && (
                              <IconButton
                                size="sm"
                                label="View schedule"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  onOpenSchedule(
                                    obj.object_instance,
                                    obj.object_name || `${formatObjectType(obj.object_type)} ${obj.object_instance}`,
                                  );
                                }}
                              >
                                <Eye size={12} strokeWidth={1.75} />
                              </IconButton>
                            )}
                          </div>
                        );
                      })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* Right panel — inspector */}
      <div style={stacked ? { ...inspectorPanel, display: detailOpen ? "block" : "none", padding: 0 } : inspectorPanel}>
        {stacked && selectedObject && (
          <div style={{ display: "flex", alignItems: "center", gap: 4, marginBottom: 8, minHeight: "var(--control-h)" }}>
            <Button variant="ghost" icon={<ChevronLeft size={16} strokeWidth={1.75} />} onClick={handleBack}>
              Objects
            </Button>
            <span
              style={{ flex: 1, minWidth: 0, textAlign: "right", fontSize: "var(--fs-sm)", color: "var(--text-dim)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
            >
              {selectedName}
            </span>
          </div>
        )}
        {selectedObject && isDeviceSelected && (
          <DeviceInspector loading={propertiesLoading} />
        )}

        {selectedObject && !isDeviceSelected && propertiesError && (
          <div
            role="alert"
            style={{ marginBottom: 8, padding: "6px 8px", fontSize: "var(--fs-xs)", color: "var(--danger)", background: "var(--danger-soft)", borderRadius: "var(--radius)" }}
          >
            {propertiesError}
          </div>
        )}

        {selectedObject && !isDeviceSelected && (
          <PropertyInspector
            objectType={selectedObject.type}
            objectInstance={selectedObject.instance}
            properties={properties}
            loading={propertiesLoading}
            onWrite={handleWrite}
            unitName={detail?.units ?? null}
            unitSymbol={detail?.units_symbol ?? null}
          />
        )}

        {!selectedObject && (
          <EmptyState
            icon={<Box size={20} strokeWidth={1.75} />}
            title="Select an object"
            description="Choose an object in the tree to read its properties."
          />
        )}
      </div>
    </div>
  );
}
