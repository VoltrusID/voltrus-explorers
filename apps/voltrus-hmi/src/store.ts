import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
// Shared engine adoption (deliberately narrow): History<T> owns the undo/redo
// stacks and newId() replaces the local newId(). Geometry (move/resize),
// z-order and clipboard stay local — HMI semantics pinned by store.test.ts
// differ from the engine's (absolute move, min-30 clamp, explicit zIndex,
// single-widget paste with +20 offset).
import { History, newId } from "@mactools/symbol-engine";
import type {
  HmiWidget,
  HmiScreen,
  WidgetType,
  PlcVariable,
  HmiTag,
  AlarmRule,
  AccessConfig,
  TranslationSet,
} from "./types";

const DEFAULT_SCREENS: HmiScreen[] = [
  {
    id: newId(),
    name: "Main Screen",
    backgroundColor: "#1e1e2e",
    widgets: [],
  },
];

const WIDGET_DEFAULTS: Record<WidgetType, { width: number; height: number }> = {
  gauge: { width: 120, height: 120 },
  button: { width: 120, height: 50 },
  indicator: { width: 80, height: 80 },
  text: { width: 150, height: 40 },
  slider: { width: 200, height: 60 },
  chart: { width: 300, height: 180 },
};

export interface Toast {
  id: number;
  kind: "success" | "error" | "info";
  message: string;
}

interface AppState {
  // Screens
  screens: HmiScreen[];
  activeScreenIndex: number;
  selectedWidgetId: string | null;

  // Unsaved-changes indicator
  isDirty: boolean;

  // Toasts (single active toast, auto-dismissed)
  toast: Toast | null;
  showToast: (kind: Toast["kind"], message: string) => void;
  dismissToast: () => void;

  // Zoom
  zoom: number;
  setZoom: (z: number) => void;

  // PLC connection
  plcIp: string;
  plcConnected: boolean;
  plcConnecting: boolean;
  plcVariables: PlcVariable[];
  setPlcIp: (ip: string) => void;
  connectPlc: () => Promise<boolean>;
  disconnectPlc: () => void;

  // Tags
  tags: HmiTag[];
  addTag: (tag: Omit<HmiTag, "id">) => void;
  removeTag: (id: string) => void;
  renameTag: (id: string, name: string) => void;

  // Theme
  theme: "dark" | "light";
  setTheme: (t: "dark" | "light") => void;

  // Screen CRUD
  addScreen: () => void;
  deleteScreen: (id: string) => void;
  setActiveScreen: (index: number) => void;
  updateScreen: (id: string, updates: Partial<HmiScreen>) => void;

  // Widget CRUD
  addWidget: (type: WidgetType, x: number, y: number) => void;
  updateWidget: (id: string, updates: Partial<HmiWidget>) => void;
  deleteWidget: (id: string) => void;
  selectWidget: (id: string | null) => void;
  moveWidget: (id: string, x: number, y: number) => void;
  resizeWidget: (id: string, width: number, height: number) => void;
  /** Push a single undo checkpoint before a continuous drag — call once at
   *  drag start; moveWidget/resizeWidget do not touch the undo stack. */
  beginWidgetDrag: () => void;
  /** Bind a variable (and its PLC data type) to a widget, e.g. on tag drop. */
  bindVariable: (id: string, variableName: string, dataType?: string) => void;
  /** Insert template widgets (undoable, selects the last inserted). */
  addTemplateWidgets: (widgets: Omit<HmiWidget, "id">[]) => void;

  // Widget layering
  bringToFront: (id: string) => void;
  sendToBack: (id: string) => void;

  // Copy/paste
  clipboard: HmiWidget | null;
  copyWidget: (id: string) => void;
  pasteWidget: () => void;

  // Export
  exportHtml: () => Promise<void>;

  // Project
  projectPath: string | null;
  saveProject: () => Promise<boolean>;
  loadProject: () => Promise<boolean>;

  // Alarms
  scadaUrl: string;
  alarms: AlarmRule[];
  setScadaUrl: (url: string) => void;
  addAlarm: (alarm: Omit<AlarmRule, "id">) => void;
  removeAlarm: (id: string) => void;
  updateAlarm: (id: string, updates: Partial<AlarmRule>) => void;

  // Access control
  accessConfig: AccessConfig | null;
  setAccessConfig: (config: AccessConfig | null) => void;

  // i18n
  translations: TranslationSet;
  defaultLanguage: string;
  addLanguage: (code: string) => void;
  removeLanguage: (code: string) => void;
  setTranslation: (key: string, langCode: string, value: string) => void;
  setDefaultLanguage: (code: string) => void;
  collectTranslatableStrings: () => string[];

  // Kiosk
  kioskMode: boolean;
  setKioskMode: (enabled: boolean) => void;

  // RPi5 deploy
  rpi5Ip: string;
  rpi5Port: number;
  setRpi5Ip: (ip: string) => void;
  setRpi5Port: (port: number) => void;
  deployToRpi5: () => Promise<void>;
  deployToEsp32: (ip: string) => Promise<void>;

  // Undo/Redo
  canUndo: boolean;
  canRedo: boolean;
  undo: () => void;
  redo: () => void;
}

// WebSocket state kept outside the store for lifecycle management
let plcWs: WebSocket | null = null;
let wsGeneration = 0;
let plcAutoReconnect = false;
let reconnectDelayMs = 1000;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let connectTimeoutTimer: ReturnType<typeof setTimeout> | null = null;

// Undo/Redo history — engine `History` owns the stacks; the coalesce window
// keeps rapid keystrokes (text fields) from flooding the 50-entry stack.
const history = new History<HmiScreen[]>(50);
let lastPushAt = 0;
const PUSH_COALESCE_MS = 400;

/** Reset undo/redo history — intended for test use only */
export function _resetUndoHistory() {
  history.clear();
  lastPushAt = 0;
}

type SetFn = (partial: Partial<AppState>) => void;

function pushUndo(screens: HmiScreen[], set: SetFn) {
  history.push(screens);
  lastPushAt = Date.now();
  set({ canUndo: true, canRedo: false, isDirty: true });
}

/**
 * Push an undo checkpoint unless one was pushed very recently (e.g. the
 * previous keystroke in a text field) — keeps typing from flooding the
 * history stack.
 */
function pushUndoCoalesced(screens: HmiScreen[], set: SetFn) {
  if (Date.now() - lastPushAt < PUSH_COALESCE_MS) {
    if (history.canRedo()) {
      // Typing must kill a stale redo tail; History has no clear-redo-only
      // op, so checkpoint instead (one extra entry in this narrow case).
      pushUndo(screens, set);
      return;
    }
    set({ isDirty: true });
    return;
  }
  pushUndo(screens, set);
}

/** Wire type codes match firmware `plc_value_type_t` (plc_bytecode.h):
 *  BOOL=0 BYTE=1 INT=2 DINT=3 REAL=4 TIME=5 STRING=6 WORD=7 DWORD=8. */
export const TYPE_MAP: Record<number, string> = {
  0: "BOOL",
  1: "BYTE",
  2: "INT",
  3: "DINT",
  4: "REAL",
  5: "TIME",
  6: "STRING",
  7: "WORD",
  8: "DWORD",
};

export function parseVarUpdate(buf: ArrayBuffer): PlcVariable[] {
  const view = new DataView(buf);
  const vars: PlcVariable[] = [];
  let offset = 0;

  if (offset + 2 > buf.byteLength) return vars;
  const count = view.getUint16(offset, true);
  offset += 2;

  for (let i = 0; i < count; i++) {
    if (offset + 2 > buf.byteLength) break;
    const nameLen = view.getUint16(offset, true);
    offset += 2;

    if (offset + nameLen > buf.byteLength) break;
    const nameBytes = new Uint8Array(buf, offset, nameLen);
    const name = new TextDecoder().decode(nameBytes);
    offset += nameLen;

    if (offset + 1 > buf.byteLength) break;
    const typeCode = view.getUint8(offset);
    offset += 1;

    if (offset + 4 > buf.byteLength) break;
    const typeName = TYPE_MAP[typeCode] ?? "UNKNOWN";

    let value: unknown;
    if (typeCode === 4) {
      // REAL: IEEE 754 float32
      value = view.getFloat32(offset, true);
    } else if (typeCode === 7 || typeCode === 8) {
      // WORD / DWORD: unsigned
      value = view.getUint32(offset, true);
    } else if (typeCode === 0) {
      value = view.getInt32(offset, true) !== 0;
    } else {
      // BYTE/INT/DINT/TIME (and STRING — firmware sends the string-table
      // index, the table itself is not part of the protocol)
      value = view.getInt32(offset, true);
    }
    offset += 4;

    // Skip 4-byte timestamp
    offset += 4;

    vars.push({ name, type: typeName, value });
  }

  return vars;
}

export const useStore = create<AppState>((set, get) => {
  let toastTimer: ReturnType<typeof setTimeout> | null = null;
  let toastId = 0;

  const showToast: AppState["showToast"] = (kind, message) => {
    if (toastTimer) clearTimeout(toastTimer);
    const id = ++toastId;
    set({ toast: { id, kind, message } });
    toastTimer = setTimeout(() => set({ toast: null }), kind === "error" ? 6000 : 4000);
  };

  return {
    screens: DEFAULT_SCREENS,
    activeScreenIndex: 0,
    selectedWidgetId: null,
    clipboard: null,
    isDirty: false,
    toast: null,
    showToast,
    dismissToast: () => {
      if (toastTimer) clearTimeout(toastTimer);
      set({ toast: null });
    },
    zoom: 1,
    setZoom: (z) => set({ zoom: Math.max(0.25, Math.min(3, z)) }),
    plcIp: "192.168.1.100",
    plcConnected: false,
    plcConnecting: false,
    plcVariables: [],
    tags: [],
    setPlcIp: (ip) => set({ plcIp: ip }),
    theme: "dark",
    setTheme: (t) => set({ theme: t }),

    connectPlc: async () => {
      const { plcIp } = get();
      const generation = ++wsGeneration;
      plcAutoReconnect = true;
      reconnectDelayMs = 1000;

      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      if (plcWs) {
        const old = plcWs;
        plcWs = null;
        old.onclose = null;
        old.close();
      }

      set({ plcConnecting: true, plcConnected: false });

      // Best-effort HTTP snapshot of current variable values.
      try {
        const resp = await fetch(`http://${plcIp}/api/variables`);
        if (generation !== wsGeneration) return false;
        if (resp.ok) {
          const data = await resp.json();
          const vars: PlcVariable[] = (data.variables ?? []).map(
            (v: { name: string; type: string; value: unknown }) => ({
              name: v.name,
              type: v.type,
              value: v.value,
            })
          );
          set({ plcVariables: vars });
        }
      } catch {
        // HTTP snapshot is best-effort
      }

      return new Promise<boolean>((resolve) => {
        let settled = false;
        const settle = (ok: boolean) => {
          if (settled) return;
          settled = true;
          if (connectTimeoutTimer) {
            clearTimeout(connectTimeoutTimer);
            connectTimeoutTimer = null;
          }
          resolve(ok);
        };

        try {
          const ws = new WebSocket(`ws://${plcIp}/ws/plc`);
          plcWs = ws;
          ws.binaryType = "arraybuffer";

          // Connect timeout: never leave the UI stuck on "Connecting...".
          connectTimeoutTimer = setTimeout(() => {
            if (generation !== wsGeneration) return;
            try { ws.close(); } catch { /* ignore */ }
            set({ plcConnected: false, plcConnecting: false });
            if (plcWs === ws) plcWs = null;
            get().showToast("error", `Cannot connect to ${plcIp} (timeout)`);
            settle(false);
          }, 8000);

          ws.onopen = () => {
            if (generation !== wsGeneration) return;
            reconnectDelayMs = 1000;
            set({ plcConnected: true, plcConnecting: false });
            settle(true);
          };

          ws.onmessage = (ev) => {
            if (generation !== wsGeneration) return;
            if (typeof ev.data === "string") return;
            const buf = ev.data as ArrayBuffer;
            if (buf.byteLength < 1) return;

            const msgType = new DataView(buf).getUint8(0);
            if (msgType !== 0x01) return;

            const parsed = parseVarUpdate(buf.slice(1));
            if (parsed.length === 0) return;

            set((state) => {
              const map = new Map(state.plcVariables.map((v) => [v.name, v]));
              for (const v of parsed) {
                const existing = map.get(v.name);
                if (existing) {
                  map.set(v.name, {
                    ...existing,
                    value: v.value,
                    type: v.type !== "UNKNOWN" ? v.type : existing.type,
                  });
                } else {
                  map.set(v.name, v);
                }
              }
              return { plcVariables: Array.from(map.values()) };
            });
          };

          ws.onclose = () => {
            if (generation !== wsGeneration) return;
            if (plcWs === ws) plcWs = null;
            set({ plcConnected: false, plcConnecting: false });
            settle(false);
            // Auto-reconnect with backoff (until the user disconnects).
            if (plcAutoReconnect) {
              reconnectTimer = setTimeout(() => {
                reconnectTimer = null;
                if (plcAutoReconnect && generation === wsGeneration) get().connectPlc();
              }, reconnectDelayMs);
              reconnectDelayMs = Math.min(reconnectDelayMs * 2, 30000);
            }
          };

          ws.onerror = () => {
            if (generation !== wsGeneration) return;
            set({ plcConnected: false, plcConnecting: false });
          };
        } catch {
          set({ plcConnected: false, plcConnecting: false });
          get().showToast("error", `Cannot connect to ${plcIp}`);
          settle(false);
        }
      });
    },

    disconnectPlc: () => {
      plcAutoReconnect = false;
      wsGeneration++;
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      if (connectTimeoutTimer) {
        clearTimeout(connectTimeoutTimer);
        connectTimeoutTimer = null;
      }
      if (plcWs) {
        const old = plcWs;
        plcWs = null;
        old.onclose = null;
        old.close();
      }
      set({ plcConnected: false, plcConnecting: false, plcVariables: [] });
    },

    addTag: (tag) => {
      const newTag: HmiTag = { ...tag, id: newId() };
      set((state) => ({ tags: [...state.tags, newTag], isDirty: true }));
    },

    removeTag: (id) => {
      set((state) => ({ tags: state.tags.filter((t) => t.id !== id), isDirty: true }));
    },

    renameTag: (id, name) => {
      set((state) => ({
        tags: state.tags.map((t) => (t.id === id ? { ...t, name } : t)),
        isDirty: true,
      }));
    },

    addScreen: () => {
      const { screens } = get();
      pushUndo(screens, set);
      const newScreen: HmiScreen = {
        id: newId(),
        name: `Screen ${screens.length + 1}`,
        backgroundColor: "#1e1e2e",
        widgets: [],
      };
      set({
        screens: [...screens, newScreen],
        activeScreenIndex: screens.length,
        selectedWidgetId: null,
      });
    },

    deleteScreen: (id) => {
      const { screens, activeScreenIndex } = get();
      if (screens.length <= 1) return;
      pushUndo(screens, set);
      const newScreens = screens.filter((s) => s.id !== id);
      const newIndex = Math.min(activeScreenIndex, newScreens.length - 1);
      set({ screens: newScreens, activeScreenIndex: newIndex, selectedWidgetId: null });
    },

    setActiveScreen: (index) => set({ activeScreenIndex: index, selectedWidgetId: null }),

    updateScreen: (id, updates) => {
      const { screens } = get();
      pushUndoCoalesced(screens, set);
      set({
        screens: screens.map((s) => (s.id === id ? { ...s, ...updates } : s)),
      });
    },

    addWidget: (type, x, y) => {
      const { screens, activeScreenIndex } = get();
      pushUndo(screens, set);
      const defaults = WIDGET_DEFAULTS[type];
      const widget: HmiWidget = {
        id: newId(),
        type,
        x,
        y,
        width: defaults.width,
        height: defaults.height,
        label: type.charAt(0).toUpperCase() + type.slice(1),
        variableName: "",
        min: 0,
        max: 100,
        color: "#22c55e",
        fontSize: 14,
      };
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: [...newScreens[activeScreenIndex].widgets, widget],
      };
      set({ screens: newScreens, selectedWidgetId: widget.id });
    },

    updateWidget: (id, updates) => {
      const { screens, activeScreenIndex } = get();
      pushUndoCoalesced(screens, set);
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: newScreens[activeScreenIndex].widgets.map((w) =>
          w.id === id ? { ...w, ...updates } : w
        ),
      };
      set({ screens: newScreens });
    },

    deleteWidget: (id) => {
      const { screens, activeScreenIndex, selectedWidgetId } = get();
      pushUndo(screens, set);
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: newScreens[activeScreenIndex].widgets.filter((w) => w.id !== id),
      };
      set({
        screens: newScreens,
        selectedWidgetId: selectedWidgetId === id ? null : selectedWidgetId,
      });
    },

    selectWidget: (id) => set({ selectedWidgetId: id }),

    moveWidget: (id, x, y) => {
      // No undo push here — beginWidgetDrag pushed one checkpoint at drag start.
      const { screens, activeScreenIndex } = get();
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: newScreens[activeScreenIndex].widgets.map((w) =>
          w.id === id ? { ...w, x, y } : w
        ),
      };
      set({ screens: newScreens });
    },

    resizeWidget: (id, width, height) => {
      const { screens, activeScreenIndex } = get();
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: newScreens[activeScreenIndex].widgets.map((w) =>
          w.id === id ? { ...w, width: Math.max(30, width), height: Math.max(30, height) } : w
        ),
      };
      set({ screens: newScreens });
    },

    beginWidgetDrag: () => {
      const { screens } = get();
      pushUndo(screens, set);
    },

    bindVariable: (id, variableName, dataType) => {
      const { screens, activeScreenIndex } = get();
      pushUndoCoalesced(screens, set);
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: newScreens[activeScreenIndex].widgets.map((w) =>
          w.id === id
            ? { ...w, variableName, dataType: dataType || w.dataType }
            : w
        ),
      };
      set({ screens: newScreens });
    },

    addTemplateWidgets: (widgets) => {
      const { screens, activeScreenIndex } = get();
      pushUndo(screens, set);
      const newWidgets: HmiWidget[] = widgets.map((w) => ({ ...w, id: newId() }));
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: [...newScreens[activeScreenIndex].widgets, ...newWidgets],
      };
      set({
        screens: newScreens,
        selectedWidgetId: newWidgets.length > 0 ? newWidgets[newWidgets.length - 1].id : null,
      });
    },

    bringToFront: (id) => {
      const { screens, activeScreenIndex } = get();
      const screen = screens[activeScreenIndex];
      const maxZ = screen.widgets.reduce((m, w) => Math.max(m, w.zIndex ?? 0), 0);
      pushUndo(screens, set);
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: newScreens[activeScreenIndex].widgets.map((w) =>
          w.id === id ? { ...w, zIndex: maxZ + 1 } : w
        ),
      };
      set({ screens: newScreens });
    },

    sendToBack: (id) => {
      const { screens, activeScreenIndex } = get();
      const screen = screens[activeScreenIndex];
      const minZ = screen.widgets.reduce((m, w) => Math.min(m, w.zIndex ?? 0), 0);
      pushUndo(screens, set);
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: newScreens[activeScreenIndex].widgets.map((w) =>
          w.id === id ? { ...w, zIndex: Math.max(0, minZ - 1) } : w
        ),
      };
      set({ screens: newScreens });
    },

    copyWidget: (id) => {
      const { screens, activeScreenIndex } = get();
      const widget = screens[activeScreenIndex].widgets.find((w) => w.id === id);
      if (widget) set({ clipboard: { ...widget } });
    },

    pasteWidget: () => {
      const { screens, activeScreenIndex, clipboard } = get();
      if (!clipboard) return;
      pushUndo(screens, set);
      const pasted: HmiWidget = {
        ...clipboard,
        id: newId(),
        x: clipboard.x + 20,
        y: clipboard.y + 20,
      };
      const newScreens = [...screens];
      newScreens[activeScreenIndex] = {
        ...newScreens[activeScreenIndex],
        widgets: [...newScreens[activeScreenIndex].widgets, pasted],
      };
      set({ screens: newScreens, selectedWidgetId: pasted.id });
    },

    exportHtml: async () => {
      const { screens, plcIp, scadaUrl, alarms, accessConfig, translations, defaultLanguage, kioskMode } = get();
      let html: string;
      try {
        html = await invoke<string>("export_hmi", {
          request: {
            screens,
            plcIp,
            scadaUrl: scadaUrl || undefined,
            alarms,
            accessConfig: accessConfig || undefined,
            translations: Object.keys(translations).length > 0 ? translations : undefined,
            defaultLanguage: defaultLanguage || undefined,
            kioskMode,
          },
        });
      } catch (e) {
        get().showToast("error", `Export failed: ${String(e)}`);
        return;
      }
      try {
        const { save } = await import("@tauri-apps/plugin-dialog");
        const path = await save({
          title: "Export HMI",
          filters: [{ name: "HTML", extensions: ["html"] }],
          defaultPath: "hmi-dashboard.html",
        });
        if (path) {
          await invoke("save_project_data", { path, data: html });
          get().showToast("success", `Exported to ${path}`);
        }
      } catch (e) {
        get().showToast("error", `Export failed: ${String(e)}`);
      }
    },

    projectPath: null,

    saveProject: async () => {
      const {
        screens,
        plcIp,
        projectPath,
        scadaUrl,
        alarms,
        accessConfig,
        translations,
        defaultLanguage,
        kioskMode,
        tags,
        rpi5Ip,
        rpi5Port,
      } = get();
      const data = JSON.stringify({
        screens,
        plcIp,
        version: "0.3",
        scadaUrl,
        alarms,
        accessConfig,
        translations,
        defaultLanguage,
        kioskMode,
        tags,
        rpi5Ip,
        rpi5Port,
      });
      try {
        if (projectPath) {
          await invoke("save_project_data", { path: projectPath, data });
        } else {
          const { save } = await import("@tauri-apps/plugin-dialog");
          const path = await save({
            title: "Save Project",
            filters: [{ name: "Voltrus HMI Project", extensions: ["vhmi"] }],
            defaultPath: "hmi-project.vhmi",
          });
          if (path) {
            await invoke("save_project_data", { path, data });
            set({ projectPath: path });
          } else {
            return false;
          }
        }
        set({ isDirty: false });
        get().showToast("success", "Project saved");
        return true;
      } catch (e) {
        get().showToast("error", `Save failed: ${String(e)}`);
        return false;
      }
    },

    loadProject: async () => {
      try {
        const { open } = await import("@tauri-apps/plugin-dialog");
        const path = await open({
          title: "Open Project",
          filters: [{ name: "Voltrus HMI Project", extensions: ["vhmi"] }],
          multiple: false,
        });
        if (path) {
          const data = await invoke<string>("load_project_data", { path });
          const project = JSON.parse(data);
          // Loading replaces the project: drop stale connection and history.
          get().disconnectPlc();
          _resetUndoHistory();
          set({
            screens: project.screens || DEFAULT_SCREENS,
            plcIp: project.plcIp || "192.168.1.100",
            projectPath: path,
            activeScreenIndex: 0,
            selectedWidgetId: null,
            scadaUrl: project.scadaUrl || "",
            alarms: project.alarms || [],
            accessConfig: project.accessConfig || null,
            translations: project.translations || {},
            defaultLanguage: project.defaultLanguage || "en",
            kioskMode: project.kioskMode || false,
            tags: project.tags || [],
            rpi5Ip: project.rpi5Ip || "",
            rpi5Port: project.rpi5Port || 8080,
            isDirty: false,
            canUndo: false,
            canRedo: false,
          });
          get().showToast("success", `Opened ${path}`);
          return true;
        }
        return false;
      } catch (e) {
        get().showToast("error", `Load failed: ${String(e)}`);
        return false;
      }
    },

    // Alarms
    scadaUrl: "",
    alarms: [],
    setScadaUrl: (url) => set({ scadaUrl: url, isDirty: true }),
    addAlarm: (alarm) => {
      const newAlarm: AlarmRule = { ...alarm, id: newId() };
      set((state) => ({ alarms: [...state.alarms, newAlarm], isDirty: true }));
    },
    removeAlarm: (id) => {
      set((state) => ({ alarms: state.alarms.filter((a) => a.id !== id), isDirty: true }));
    },
    updateAlarm: (id, updates) => {
      set((state) => ({
        alarms: state.alarms.map((a) => (a.id === id ? { ...a, ...updates } : a)),
        isDirty: true,
      }));
    },

    // Access control
    accessConfig: null,
    setAccessConfig: (config) => set({ accessConfig: config, isDirty: true }),

    // i18n
    translations: {},
    defaultLanguage: "en",
    addLanguage: (code) => {
      set((state) => {
        if (state.translations[code]) return state;
        const newTranslations = { ...state.translations, [code]: {} };
        return { translations: newTranslations, isDirty: true };
      });
    },
    removeLanguage: (code) => {
      set((state) => {
        const { [code]: _, ...rest } = state.translations;
        // Never leave the default pointing at a removed language.
        const defaultLanguage =
          state.defaultLanguage === code
            ? Object.keys(rest)[0] ?? "en"
            : state.defaultLanguage;
        return { translations: rest, defaultLanguage, isDirty: true };
      });
    },
    setTranslation: (key, langCode, value) => {
      set((state) => ({
        translations: {
          ...state.translations,
          [langCode]: {
            ...state.translations[langCode],
            [key]: value,
          },
        },
        isDirty: true,
      }));
    },
    setDefaultLanguage: (code) => set({ defaultLanguage: code, isDirty: true }),
    collectTranslatableStrings: () => {
      const { screens, alarms } = get();
      const strings = new Set<string>();
      for (const screen of screens) {
        strings.add(screen.name);
        for (const widget of screen.widgets) {
          if (widget.label) strings.add(widget.label);
        }
      }
      for (const alarm of alarms) {
        if (alarm.message) strings.add(alarm.message);
      }
      return Array.from(strings);
    },

    // Kiosk
    kioskMode: false,
    setKioskMode: (enabled) => set({ kioskMode: enabled, isDirty: true }),

    // RPi5 deploy
    rpi5Ip: "",
    rpi5Port: 8080,
    setRpi5Ip: (ip) => set({ rpi5Ip: ip, isDirty: true }),
    setRpi5Port: (port) => set({ rpi5Port: Number.isFinite(port) ? port : 8080, isDirty: true }),
    deployToRpi5: async () => {
      const { rpi5Ip, rpi5Port, screens, plcIp, scadaUrl, alarms, accessConfig, translations, defaultLanguage, kioskMode } = get();
      try {
        const html = await invoke<string>("export_hmi", {
          request: {
            screens,
            plcIp,
            scadaUrl: scadaUrl || undefined,
            alarms,
            accessConfig: accessConfig || undefined,
            translations: Object.keys(translations).length > 0 ? translations : undefined,
            defaultLanguage: defaultLanguage || undefined,
            kioskMode,
          },
        });
        await invoke<string>("deploy_to_rpi5", { ip: rpi5Ip, port: rpi5Port, html });
        get().showToast("success", `Deployed to RPi5 at ${rpi5Ip}:${rpi5Port}`);
      } catch (e) {
        get().showToast("error", `RPi5 deploy failed: ${String(e)}`);
        throw e;
      }
    },
    deployToEsp32: async (ip) => {
      const { screens, plcIp, scadaUrl, alarms, accessConfig, translations, defaultLanguage, kioskMode } = get();
      try {
        const html = await invoke<string>("export_hmi", {
          request: {
            screens,
            plcIp,
            scadaUrl: scadaUrl || undefined,
            alarms,
            accessConfig: accessConfig || undefined,
            translations: Object.keys(translations).length > 0 ? translations : undefined,
            defaultLanguage: defaultLanguage || undefined,
            kioskMode,
          },
        });
        await invoke<string>("deploy_to_esp32", { ip, html });
        get().showToast("success", `Deployed to ESP32 at ${ip}`);
      } catch (e) {
        get().showToast("error", `ESP32 deploy failed: ${String(e)}`);
        throw e;
      }
    },

    // Undo/Redo
    canUndo: false,
    canRedo: false,
    undo: () => {
      const { screens } = get();
      const prev = history.undo(screens);
      if (!prev) return;
      set({
        screens: prev,
        selectedWidgetId: null,
        activeScreenIndex: Math.min(get().activeScreenIndex, prev.length - 1),
        canUndo: history.canUndo(),
        canRedo: history.canRedo(),
        isDirty: true,
      });
    },
    redo: () => {
      const { screens } = get();
      const next = history.redo(screens);
      if (!next) return;
      set({
        screens: next,
        selectedWidgetId: null,
        activeScreenIndex: Math.min(get().activeScreenIndex, next.length - 1),
        canUndo: history.canUndo(),
        canRedo: history.canRedo(),
        isDirty: true,
      });
    },
  };
});
