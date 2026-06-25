import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
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

export function uid(): string {
  return Math.random().toString(36).substring(2, 10);
}

const DEFAULT_SCREENS: HmiScreen[] = [
  {
    id: uid(),
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

interface AppState {
  // Screens
  screens: HmiScreen[];
  activeScreenIndex: number;
  selectedWidgetId: string | null;

  // Zoom
  zoom: number;
  setZoom: (z: number) => void;

  // PLC connection
  plcIp: string;
  plcConnected: boolean;
  plcConnecting: boolean;
  plcVariables: PlcVariable[];
  setPlcIp: (ip: string) => void;
  connectPlc: () => Promise<void>;
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
  saveProject: () => Promise<void>;
  loadProject: () => Promise<void>;

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

  // Undo/Redo
  canUndo: boolean;
  canRedo: boolean;
  undo: () => void;
  redo: () => void;
}

// WebSocket reference kept outside store for lifecycle management
let plcWs: WebSocket | null = null;

// Undo/Redo history
const MAX_HISTORY = 50;
const undoStack: string[] = [];
const redoStack: string[] = [];

/** Reset undo/redo stacks — intended for test use only */
export function _resetUndoHistory() {
  undoStack.length = 0;
  redoStack.length = 0;
}

function pushUndo(screens: HmiScreen[], set: (partial: Partial<AppState>) => void) {
  undoStack.push(JSON.stringify(screens));
  if (undoStack.length > MAX_HISTORY) undoStack.shift();
  redoStack.length = 0;
  set({ canUndo: true, canRedo: false });
}

export const TYPE_MAP: Record<number, string> = {
  0: "BOOL",
  1: "INT",
  2: "DINT",
  3: "REAL",
  4: "WORD",
  5: "DWORD",
  6: "STRING",
  7: "BYTE",
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
    if (typeName === "REAL") {
      // IEEE 754 float sent by firmware
      value = view.getFloat32(offset, true);
    } else {
      const rawValue = view.getInt32(offset, true);
      if (typeName === "BOOL") value = rawValue !== 0;
      else if (typeName === "STRING") value = String(rawValue);
      else value = rawValue;
    }
    offset += 4;

    // Skip 4-byte timestamp
    offset += 4;

    vars.push({ name, type: typeName, value });
  }

  return vars;
}

export const useStore = create<AppState>((set, get) => ({
  screens: DEFAULT_SCREENS,
  activeScreenIndex: 0,
  selectedWidgetId: null,
  clipboard: null,
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
    if (plcWs) {
      plcWs.close();
      plcWs = null;
    }

    set({ plcConnecting: true, plcConnected: false });

    try {
      const resp = await fetch(`http://${plcIp}/api/variables`);
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

    return new Promise<void>((resolve) => {
      try {
        const ws = new WebSocket(`ws://${plcIp}/ws/plc`);
        plcWs = ws;
        ws.binaryType = "arraybuffer";

        ws.onopen = () => {
          set({ plcConnected: true, plcConnecting: false });
          resolve();
        };

        ws.onmessage = (ev) => {
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
          plcWs = null;
          set({ plcConnected: false, plcConnecting: false });
          resolve();
        };

        ws.onerror = () => {
          plcWs = null;
          set({ plcConnected: false, plcConnecting: false });
          resolve();
        };
      } catch {
        set({ plcConnected: false, plcConnecting: false });
        resolve();
      }
    });
  },

  disconnectPlc: () => {
    if (plcWs) {
      plcWs.close();
      plcWs = null;
    }
    set({ plcConnected: false, plcConnecting: false, plcVariables: [] });
  },

  addTag: (tag) => {
    const newTag: HmiTag = { ...tag, id: uid() };
    set((state) => ({ tags: [...state.tags, newTag] }));
  },

  removeTag: (id) => {
    set((state) => ({ tags: state.tags.filter((t) => t.id !== id) }));
  },

  renameTag: (id, name) => {
    set((state) => ({
      tags: state.tags.map((t) => (t.id === id ? { ...t, name } : t)),
    }));
  },

  addScreen: () => {
    const { screens } = get();
    pushUndo(screens, set);
    const newScreen: HmiScreen = {
      id: uid(),
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
    pushUndo(screens, set);
    if (screens.length <= 1) return;
    const newScreens = screens.filter((s) => s.id !== id);
    const newIndex = Math.min(activeScreenIndex, newScreens.length - 1);
    set({ screens: newScreens, activeScreenIndex: newIndex, selectedWidgetId: null });
  },

  setActiveScreen: (index) => set({ activeScreenIndex: index, selectedWidgetId: null }),

  updateScreen: (id, updates) => {
    const { screens } = get();
    pushUndo(screens, set);
    set({
      screens: screens.map((s) => (s.id === id ? { ...s, ...updates } : s)),
    });
  },

  addWidget: (type, x, y) => {
    const { screens, activeScreenIndex } = get();
    pushUndo(screens, set);
    const defaults = WIDGET_DEFAULTS[type];
    const widget: HmiWidget = {
      id: uid(),
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
    pushUndo(screens, set);
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
    const { screens, activeScreenIndex } = get();
    pushUndo(screens, set);
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
    pushUndo(screens, set);
    const newScreens = [...screens];
    newScreens[activeScreenIndex] = {
      ...newScreens[activeScreenIndex],
      widgets: newScreens[activeScreenIndex].widgets.map((w) =>
        w.id === id ? { ...w, width: Math.max(30, width), height: Math.max(30, height) } : w
      ),
    };
    set({ screens: newScreens });
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
      id: uid(),
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
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        title: "Export HMI",
        filters: [{ name: "HTML", extensions: ["html"] }],
        defaultPath: "hmi-dashboard.html",
      });
      if (path) {
        await invoke("save_project_data", { path, data: html });
      }
    } catch (e) {
      console.error("Export failed:", e);
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
    } = get();
    const data = JSON.stringify({
      screens,
      plcIp,
      version: "0.2",
      scadaUrl,
      alarms,
      accessConfig,
      translations,
      defaultLanguage,
      kioskMode,
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
        }
      }
    } catch (e) {
      console.error("Save failed:", e);
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
        });
      }
    } catch (e) {
      console.error("Load failed:", e);
    }
  },

  // Alarms
  scadaUrl: "",
  alarms: [],
  setScadaUrl: (url) => set({ scadaUrl: url }),
  addAlarm: (alarm) => {
    const newAlarm: AlarmRule = { ...alarm, id: uid() };
    set((state) => ({ alarms: [...state.alarms, newAlarm] }));
  },
  removeAlarm: (id) => {
    set((state) => ({ alarms: state.alarms.filter((a) => a.id !== id) }));
  },
  updateAlarm: (id, updates) => {
    set((state) => ({
      alarms: state.alarms.map((a) => (a.id === id ? { ...a, ...updates } : a)),
    }));
  },

  // Access control
  accessConfig: null,
  setAccessConfig: (config) => set({ accessConfig: config }),

  // i18n
  translations: {},
  defaultLanguage: "en",
  addLanguage: (code) => {
    set((state) => {
      if (state.translations[code]) return state;
      const newTranslations = { ...state.translations, [code]: {} };
      return { translations: newTranslations };
    });
  },
  removeLanguage: (code) => {
    set((state) => {
      const { [code]: _, ...rest } = state.translations;
      return { translations: rest };
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
    }));
  },
  setDefaultLanguage: (code) => set({ defaultLanguage: code }),
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
  setKioskMode: (enabled) => set({ kioskMode: enabled }),

  // RPi5 deploy
  rpi5Ip: "",
  rpi5Port: 8080,
  setRpi5Ip: (ip) => set({ rpi5Ip: ip }),
  setRpi5Port: (port) => set({ rpi5Port: port }),
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
      const result = await invoke<string>("deploy_to_rpi5", {
        ip: rpi5Ip,
        port: rpi5Port,
        html,
      });
      console.log("RPi5 deploy:", result);
    } catch (e) {
      console.error("RPi5 deploy failed:", e);
      throw e;
    }
  },

  // Undo/Redo
  canUndo: false,
  canRedo: false,
  undo: () => {
    const { screens } = get();
    if (undoStack.length === 0) return;
    redoStack.push(JSON.stringify(screens));
    const prev = JSON.parse(undoStack.pop()!);
    set({ screens: prev, selectedWidgetId: null, canUndo: undoStack.length > 0, canRedo: true });
  },
  redo: () => {
    const { screens } = get();
    if (redoStack.length === 0) return;
    undoStack.push(JSON.stringify(screens));
    const next = JSON.parse(redoStack.pop()!);
    set({ screens: next, selectedWidgetId: null, canUndo: true, canRedo: redoStack.length > 0 });
  },
}));
