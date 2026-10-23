import { useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  ConnectionForm,
  DataTable,
  StatusBar,
  Toolbar,
  LogPanel,
  ValueEditor,
  Section,
  Segmented,
  Tabs,
  Field,
  EmptyState,
  Badge,
  Button,
  IconButton,
  Dialog,
  ProBadge,
  ProAboutRow,
  useProStatus,
  saveTextFile,
  SheetSelect,
} from "@mactools/ui";
import type { LogEntry, SheetOption } from "@mactools/ui";
import {
  Trash2,
  Plus,
  X,
  ChevronDown,
  ChevronRight,
  RadioTower,
  Braces,
} from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import { TopicTree, type TreeEntry } from "./TopicTree";
import { loadProfiles, upsertProfile, deleteProfile, FREE_PROFILE_LIMIT, type ConnProfile } from "./profiles";
import { messagesToCsv } from "./csv";
import { ChartPanel } from "./ChartPanel";
import { SeriesStore } from "./charts";
import { DecoderDialog } from "./DecoderDialog";
import { ClearRetainedDialog, PayloadInspector } from "./Dialogs";
import {
  formatPayload,
  payloadBytes,
  payloadInfoFromEvent,
  type Decoded,
  type PayloadFormat,
  type PayloadInfo,
} from "./payload";
import { EMPTY_PROTO, loadProto, saveProto, type ProtoSettings } from "./protoStore";
import {
  applyMessage,
  clearData,
  markRetainedCleared,
  newSession,
  nextSessionId,
  retainedTopics,
  DEFAULT_FORM,
  type ClientMsg,
  type FormSnapshot,
  type Redecoded,
  type Session,
} from "./sessions";

interface MqttConfig {
  host: string;
  port: number;
  client_id: string;
  username?: string;
  password?: string;
  tls?: {
    enabled: boolean;
    ca_pem?: string;
    client_cert_pem?: string;
    client_key_pem?: string;
  };
  keep_alive_secs?: number;
  last_will?: {
    topic: string;
    payload: string;
    qos: string;
    retain: boolean;
  };
}

interface TopicInfo extends PayloadInfo {
  topic: string;
  last_payload: string;
  raw_hex?: string;
  last_update: number;
  message_count: number;
  retain: boolean;
}

/** Backend `client-message` event. */
interface ClientMessageEvent {
  conn: string;
  topic: string;
  payload: string;
  raw_hex?: string;
  decoded?: Decoded;
  qos: number;
  retain: boolean;
}

/** Backend `broker-message` event. */
interface BrokerMessageEvent {
  topic: string;
  payload: string;
  raw_hex?: string;
  decoded?: Decoded;
  timestamp: number;
  retain: boolean;
}

interface BrokerMsg extends PayloadInfo {
  topic: string;
  retain: boolean;
  ts: number;
}

const monoFont: React.CSSProperties = { fontFamily: "var(--font-mono)" };

const sidebarStyle: React.CSSProperties = {
  background: "var(--surface)",
  padding: 0,
  overflowY: "auto",
  display: "flex",
  flexDirection: "column",
};

const railStyle: React.CSSProperties = {
  background: "var(--surface)",
  padding: 0,
  overflowY: "auto",
  display: "flex",
  flexDirection: "column",
};

const payloadCellStyle: React.CSSProperties = {
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  fontFamily: "var(--font-mono)",
  fontSize: "var(--fs-xs)",
};

const QOS_OPTIONS = [
  { value: "atmostonce", label: "QoS 0" },
  { value: "atleastonce", label: "QoS 1" },
  { value: "exactlyonce", label: "QoS 2" },
];

function App() {
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [mode, setMode] = useState<"client" | "broker">("client");
  const [log, setLog] = useState<LogEntry[]>([]);
  const [editingTopic, setEditingTopic] = useState<string | null>(null);
  const [showInfo, setShowInfo] = useState(false);

  // Client connections: several can be open at once (Pro), one is shown at a time.
  const [sessions, setSessions] = useState<Session[]>(() => [newSession("c1")]);
  const [activeId, setActiveId] = useState("c1");
  const active = sessions.find((s) => s.id === activeId) ?? sessions[0];
  const clientConnected = active.status === "connected";
  const clientLoading = active.status === "connecting";
  const clientError = active.error;
  const subscriptions = active.subscriptions;
  const clientMessages = active.messages;
  const clientTopicValues = active.topicValues;
  const blinkingClientTopics = active.blinking;
  const clientEndpoint = active.endpoint;
  const clientRtt = active.rtt;
  const [subscribeTopic, setSubscribeTopic] = useState("");
  const [publishTopic, setPublishTopic] = useState("");
  const [publishPayload, setPublishPayload] = useState("");

  // Numeric charts (Pro), Sparkplug B / protobuf decoding (Pro), retained clearing (Pro).
  const [chartStore] = useState(() => new SeriesStore());
  const proRef = useRef(pro);
  proRef.current = pro;
  const [chartSelected, setChartSelected] = useState<Record<string, string[]>>({});
  const [protoSettings, setProtoSettings] = useState<ProtoSettings>(EMPTY_PROTO);
  const [decoderOpen, setDecoderOpen] = useState(false);
  const [inspecting, setInspecting] = useState<{
    topic: string;
    info: PayloadInfo & { decoded: Decoded };
  } | null>(null);
  const [retainedOpen, setRetainedOpen] = useState(false);
  const [subscribeQos, setSubscribeQos] = useState<string>("atmostonce");
  const [publishQos, setPublishQos] = useState<string>("atmostonce");
  const [publishRetained, setPublishRetained] = useState(false);

  // Data panel view + payload rendering.
  const [dataView, setDataView] = useState<"topics" | "tree" | "history" | "charts">("topics");
  const [payloadFormat, setPayloadFormat] = useState<PayloadFormat>("raw");

  // Connection-URL paste + endpoint shown in the status bar.
  const [connUrl, setConnUrl] = useState("");
  const [formKey, setFormKey] = useState(0);
  const [parsedInit, setParsedInit] = useState<Record<string, string | number> | undefined>(undefined);

  // Saved connection profiles (persisted) + the live form-field snapshot.
  const [profiles, setProfiles] = useState<ConnProfile[]>([]);
  const [selectedProfile, setSelectedProfile] = useState("");
  const [formValues, setFormValues] = useState<Record<string, string | number>>({});

  // Broker message history (broker mode already had per-topic; this adds a stream).
  const [brokerMessages, setBrokerMessages] = useState<BrokerMsg[]>([]);

  // Client advanced (TLS / keep-alive / LWT)
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [showCerts, setShowCerts] = useState(false);
  const [showWill, setShowWill] = useState(false);
  const [saveAsOpen, setSaveAsOpen] = useState(false);
  const [saveAsName, setSaveAsName] = useState("");
  const [clientTlsEnabled, setClientTlsEnabled] = useState(false);
  const [clientCa, setClientCa] = useState("");
  const [clientCert, setClientCert] = useState("");
  const [clientKey, setClientKey] = useState("");
  const [keepAlive, setKeepAlive] = useState(30);
  const [lwtTopic, setLwtTopic] = useState("");
  const [lwtPayload, setLwtPayload] = useState("");
  const [lwtQos, setLwtQos] = useState("atmostonce");
  const [lwtRetain, setLwtRetain] = useState(false);

  // Broker state
  const [brokerRunning, setBrokerRunning] = useState(false);
  const [brokerPort, setBrokerPort] = useState(1883);
  const [brokerMaxConn, setBrokerMaxConn] = useState(100);
  const [brokerTopics, setBrokerTopics] = useState<TopicInfo[]>([]);
  // Topics whose latest value changed recently — used to blink their row.
  const [blinkingTopics, setBlinkingTopics] = useState<Set<string>>(new Set());
  const lastTopicUpdateRef = useRef<Map<string, number>>(new Map());
  const [brokerWsPort, setBrokerWsPort] = useState<number | "">("");
  const [brokerAuthEnabled, setBrokerAuthEnabled] = useState(false);
  const [brokerUsers, setBrokerUsers] = useState<{ username: string; password: string }[]>([
    { username: "", password: "" },
  ]);
  const [brokerTlsEnabled, setBrokerTlsEnabled] = useState(false);
  const [brokerTlsCert, setBrokerTlsCert] = useState("");
  const [brokerTlsKey, setBrokerTlsKey] = useState("");
  const [brokerTlsCa, setBrokerTlsCa] = useState("");

  // Panel resize state
  const [leftWidth, setLeftWidth] = useState(260);
  const [rightWidth, setRightWidth] = useState(220);
  const [tableHeight, setTableHeight] = useState(300);
  const [dragSide, setDragSide] = useState<"left" | "right" | "table" | null>(null);

  // Mobile detection
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [mobileTab, setMobileTab] = useState<"connect" | "data" | "topics" | "log">("connect");

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev.slice(-499), { timestamp: Date.now(), direction, message }]);
  }, []);

  // Panel resize handlers
  const dragStartRef = useRef({ startX: 0, startY: 0, startWidth: 0, startHeight: 0 });
  const handleDragStart = useCallback((side: "left" | "right" | "table", e: React.MouseEvent) => {
    e.preventDefault();
    dragStartRef.current = {
      startX: e.clientX, startY: e.clientY,
      startWidth: side === "left" ? leftWidth : side === "right" ? rightWidth : 0,
      startHeight: side === "table" ? tableHeight : 0,
    };
    setDragSide(side);
  }, [leftWidth, rightWidth, tableHeight]);

  useEffect(() => {
    if (!dragSide) return;
    const onMove = (e: MouseEvent) => {
      if (dragSide === "left") {
        const dx = e.clientX - dragStartRef.current.startX;
        setLeftWidth(Math.max(180, Math.min(400, dragStartRef.current.startWidth + dx)));
      } else if (dragSide === "right") {
        const dx = e.clientX - dragStartRef.current.startX;
        setRightWidth(Math.max(160, Math.min(400, dragStartRef.current.startWidth - dx)));
      } else if (dragSide === "table") {
        const dy = e.clientY - dragStartRef.current.startY;
        setTableHeight(Math.max(80, dragStartRef.current.startHeight + dy));
      }
    };
    const onUp = () => setDragSide(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
  }, [dragSide]);

  // Load saved connection profiles once on mount.
  useEffect(() => {
    setProfiles(loadProfiles());
  }, []);

  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    if (clientError) addLog("error", clientError);
  }, [clientError, addLog]);

  const updateSession = useCallback((id: string, fn: (s: Session) => Session) => {
    setSessions((prev) => prev.map((s) => (s.id === id ? fn(s) : s)));
  }, []);

  // Blink a client topic briefly when its value changes.
  const blinkClientTopic = useCallback((conn: string, topic: string) => {
    setSessions((prev) =>
      prev.map((s) =>
        s.id !== conn || s.blinking.has(topic) ? s : { ...s, blinking: new Set(s.blinking).add(topic) },
      ),
    );
    window.setTimeout(() => {
      setSessions((prev) =>
        prev.map((s) => {
          if (s.id !== conn || !s.blinking.has(topic)) return s;
          const blinking = new Set(s.blinking);
          blinking.delete(topic);
          return { ...s, blinking };
        }),
      );
    }, 600);
  }, []);

  // Single ingest path for every received client message: append to history,
  // update latest-value-per-topic with a per-topic count, feed the charts, and blink.
  const ingestClientMessage = useCallback(
    (m: ClientMessageEvent) => {
      if (!m.topic) return;
      const info = payloadInfoFromEvent(m);
      const ts = Date.now();
      setSessions((prev) =>
        prev.map((s) =>
          s.id === m.conn ? applyMessage(s, { topic: m.topic, qos: m.qos ?? 0, retain: !!m.retain, ts, ...info }) : s,
        ),
      );
      blinkClientTopic(m.conn, m.topic);
      if (proRef.current) chartStore.ingest(m.conn, m.topic, info.payload, info.decoded, ts);
    },
    [blinkClientTopic, chartStore],
  );

  // Event: listen for client messages and disconnects.
  useEffect(() => {
    let unlistenClient: UnlistenFn | null = null;
    let unlistenDisconnect: UnlistenFn | null = null;
    let unlistenRtt: UnlistenFn | null = null;

    listen<ClientMessageEvent>("client-message", (event) => {
      ingestClientMessage(event.payload);
    }).then((fn) => { unlistenClient = fn; });

    listen<{ conn: string; rtt: number }>("client-rtt", (event) => {
      updateSession(event.payload.conn, (s) => ({ ...s, rtt: event.payload.rtt }));
    }).then((fn) => { unlistenRtt = fn; });

    listen<{ conn: string; reason: string; detail?: string }>("client-disconnected", (event) => {
      const { conn, detail, reason } = event.payload;
      updateSession(conn, (s) => ({
        ...s,
        status: "idle",
        rtt: null,
        endpoint: null,
        subscriptions: [],
        error: `Connection lost: ${detail ?? reason}`,
      }));
    }).then((fn) => { unlistenDisconnect = fn; });

    return () => {
      unlistenClient?.();
      unlistenDisconnect?.();
      unlistenRtt?.();
    };
  }, [ingestClientMessage, updateSession]);

  // Poll topics list periodically (less critical, backed by command)
  useEffect(() => {
    if (!brokerRunning) return;
    const interval = setInterval(async () => {
      try {
        const topics = await invoke<TopicInfo[]>("broker_get_topics");
        setBrokerTopics(topics);
      } catch { }
    }, 1000);
    return () => clearInterval(interval);
  }, [brokerRunning]);

  // Detect topic value changes (via last_update) and blink the row briefly.
  useEffect(() => {
    if (!brokerRunning) {
      setBlinkingTopics(new Set());
      lastTopicUpdateRef.current.clear();
      return;
    }
    for (const t of brokerTopics) {
      const prev = lastTopicUpdateRef.current.get(t.topic);
      if (prev !== undefined && prev !== t.last_update) {
        setBlinkingTopics((cur) => {
          const next = new Set(cur);
          next.add(t.topic);
          return next;
        });
        const topic = t.topic;
        window.setTimeout(() => {
          setBlinkingTopics((cur) => {
            if (!cur.has(topic)) return cur;
            const next = new Set(cur);
            next.delete(topic);
            return next;
          });
        }, 600);
      }
      lastTopicUpdateRef.current.set(t.topic, t.last_update);
    }
  }, [brokerTopics, brokerRunning]);

  // Reconcile local `brokerRunning` flag with backend truth. Catches cases
  // where the rumqttd task exited (port in use, panic, etc.) so the UI
  // doesn't claim "running" when it isn't.
  useEffect(() => {
    if (!brokerRunning) return;
    const interval = setInterval(async () => {
      try {
        // BrokerStatus serializes as "running" | "starting" | "stopped"
        // or { "error": "<msg>" } for the Error variant.
        const status = await invoke<string | { error: string }>("broker_get_status");
        const alive = status === "running" || status === "starting";
        if (!alive) {
          setBrokerRunning(false);
          setBrokerTopics([]);
          setBlinkingTopics(new Set());
          lastTopicUpdateRef.current.clear();
          const detail =
            typeof status === "object" && status !== null
              ? status.error
              : typeof status === "string"
                ? status
                : "unknown";
          addLog("error", `Broker stopped${detail && detail !== "stopped" ? `: ${detail}` : ""}`);
        }
      } catch {
        // Command/state failure — leave the local flag as-is this tick.
      }
    }, 2000);
    return () => clearInterval(interval);
  }, [brokerRunning, addLog]);

  // Broker message stream — feeds the History view (broker mode).
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<BrokerMessageEvent>("broker-message", (event) => {
      const ts = event.payload.timestamp ?? Date.now();
      const info = payloadInfoFromEvent(event.payload);
      setBrokerMessages((prev) => [
        ...prev.slice(-499),
        { ...info, topic: String(event.payload.topic), retain: !!event.payload.retain, ts },
      ]);
      if (proRef.current) chartStore.ingest("broker", event.payload.topic, info.payload, info.decoded, ts);
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [chartStore]);

  // Apply the saved protobuf schema once Pro is known to be active.
  useEffect(() => {
    if (!pro) return;
    const saved = loadProto();
    if (!saved.source.trim()) return;
    invoke("proto_configure", { source: saved.source, rules: saved.rules })
      .then(() => setProtoSettings(saved))
      .catch((e) => addLog("error", `Saved protobuf schema was not applied: ${String(e)}`));
  }, [pro, addLog]);



  // ── Client handlers ──
  const captureForm = useCallback(
    (): FormSnapshot => ({
      // The form reports values only after an edit; until then the session still holds them.
      values: Object.keys(formValues).length ? formValues : active.form.values,
      tls: clientTlsEnabled,
      ca: clientCa,
      cert: clientCert,
      key: clientKey,
      keepAlive,
      lwtTopic,
      lwtPayload,
      lwtQos,
      lwtRetain,
    }),
    [formValues, active.form.values, clientTlsEnabled, clientCa, clientCert, clientKey, keepAlive, lwtTopic, lwtPayload, lwtQos, lwtRetain],
  );

  const applyForm = useCallback((f: FormSnapshot) => {
    setParsedInit(f.values);
    setFormValues(f.values);
    setFormKey((k) => k + 1);
    setClientTlsEnabled(f.tls);
    setClientCa(f.ca);
    setClientCert(f.cert);
    setClientKey(f.key);
    setKeepAlive(f.keepAlive);
    setLwtTopic(f.lwtTopic);
    setLwtPayload(f.lwtPayload);
    setLwtQos(f.lwtQos);
    setLwtRetain(f.lwtRetain);
    setSelectedProfile("");
  }, []);

  const selectSession = useCallback(
    (id: string) => {
      if (id === activeId) return;
      const next = sessions.find((s) => s.id === id);
      if (!next) return;
      const leaving = captureForm();
      updateSession(activeId, (s) => ({ ...s, form: leaving }));
      applyForm(next.form);
      setActiveId(id);
    },
    [activeId, sessions, captureForm, applyForm, updateSession],
  );

  const addSession = useCallback(() => {
    if (!requirePro("Several simultaneous connections")) return;
    const id = nextSessionId(sessions);
    const fresh = newSession(id);
    // Two connections to one broker need different client ids or the broker drops the older one.
    fresh.form = { ...DEFAULT_FORM, values: { ...DEFAULT_FORM.values, clientId: `mactools-explorer-${id}` } };
    const leaving = captureForm();
    setSessions((prev) => [...prev.map((s) => (s.id === activeId ? { ...s, form: leaving } : s)), fresh]);
    applyForm(fresh.form);
    setActiveId(id);
  }, [requirePro, sessions, activeId, captureForm, applyForm]);

  const closeSession = useCallback(
    async (id: string) => {
      if (!sessions.some((s) => s.id === id)) return;
      try {
        await invoke("client_disconnect", { conn: id });
      } catch { /* not connected */ }
      chartStore.clear(id);
      const rest = sessions.filter((s) => s.id !== id);
      if (rest.length === 0) {
        const fresh = newSession("c1");
        setSessions([fresh]);
        setActiveId("c1");
        applyForm(fresh.form);
        return;
      }
      setSessions(rest);
      if (id === activeId) {
        applyForm(rest[0].form);
        setActiveId(rest[0].id);
      }
    },
    [sessions, activeId, chartStore, applyForm],
  );

  const handleClientConnect = useCallback(
    async (values: Record<string, string | number>) => {
      const conn = active.id;
      const config: MqttConfig = {
        host: String(values.host),
        port: Number(values.port),
        client_id: String(values.clientId || "voltrus-mqtt-explorer"),
        username: values.username ? String(values.username) : undefined,
        password: values.password ? String(values.password) : undefined,
        tls: {
          enabled: clientTlsEnabled,
          // Custom CA, mTLS and Last Will are Pro: only send them when unlocked.
          ca_pem: (pro && clientCa.trim()) || undefined,
          client_cert_pem: (pro && clientCert.trim()) || undefined,
          client_key_pem: (pro && clientKey.trim()) || undefined,
        },
        keep_alive_secs: Number(keepAlive) || 30,
        last_will: pro && lwtTopic.trim()
          ? { topic: lwtTopic.trim(), payload: lwtPayload, qos: lwtQos, retain: lwtRetain }
          : undefined,
      };
      const clash = sessions.some(
        (s) =>
          s.id !== conn &&
          s.status !== "idle" &&
          s.clientId === config.client_id &&
          s.endpoint?.ip === config.host &&
          s.endpoint.port === config.port,
      );
      if (clash) {
        updateSession(conn, (s) => ({
          ...s,
          error: "Another open connection to this broker already uses this Client ID. The broker would drop it. Change the Client ID.",
        }));
        return;
      }
      const form = captureForm();
      // Fresh session: clear any data from a previous connection.
      chartStore.clear(conn);
      updateSession(conn, (s) => ({
        ...clearData(s),
        form,
        status: "connecting",
        error: null,
        clientId: config.client_id,
        endpoint: { ip: config.host, port: config.port },
      }));
      try {
        await invoke("client_connect", { conn, config });
        updateSession(conn, (s) => ({
          ...s,
          status: "connected",
          label: selectedProfile || `${config.host}:${config.port}`,
        }));
        addLog("info", `Connected to ${config.host}:${config.port}`);
      } catch (e) {
        updateSession(conn, (s) => ({ ...s, status: "idle", error: String(e), endpoint: null, clientId: null }));
        addLog("error", `Connection failed: ${String(e)}`);
      }
    },
    [addLog, pro, active.id, sessions, selectedProfile, captureForm, chartStore, updateSession, clientTlsEnabled, clientCa, clientCert, clientKey, keepAlive, lwtTopic, lwtPayload, lwtQos, lwtRetain],
  );

  const handleClientDisconnect = useCallback(async () => {
    const conn = active.id;
    try {
      await invoke("client_disconnect", { conn });
    } catch { /* ignore */ }
    updateSession(conn, (s) => ({ ...s, status: "idle", endpoint: null, clientId: null, rtt: null, subscriptions: [] }));
    addLog("info", "Disconnected");
  }, [active.id, addLog, updateSession]);

  const handleSubscribe = useCallback(async () => {
    if (!subscribeTopic || !clientConnected) return;
    if (subscriptions.some((s) => s.topic === subscribeTopic)) return;
    const conn = active.id;
    try {
      // Backend now awaits the broker's SUBACK and returns the granted QoS
      // (the broker may downgrade it).
      const reqQos = subscribeQos === "atleastonce" ? 1 : subscribeQos === "exactlyonce" ? 2 : 0;
      const granted = await invoke<number>("client_subscribe", { conn, topic: subscribeTopic, qos: subscribeQos });
      const qosNum = String(granted);
      const note = granted < reqQos ? " (downgraded by broker)" : "";
      updateSession(conn, (s) => ({ ...s, subscriptions: [...s.subscriptions, { topic: subscribeTopic, qos: qosNum }] }));
      addLog("tx", `Subscribed: ${subscribeTopic} (QoS ${qosNum}${note})`);
      setSubscribeTopic("");
    } catch (e) {
      addLog("error", `Subscribe failed: ${String(e)}`);
    }
  }, [subscribeTopic, clientConnected, subscriptions, subscribeQos, active.id, addLog, updateSession]);

  const handleUnsubscribe = useCallback(async (topic: string) => {
    const conn = active.id;
    try {
      await invoke("client_unsubscribe", { conn, topic });
      updateSession(conn, (s) => ({ ...s, subscriptions: s.subscriptions.filter((x) => x.topic !== topic) }));
      addLog("tx", `Unsubscribed: ${topic}`);
    } catch (e) {
      addLog("error", `Unsubscribe failed: ${String(e)}`);
    }
  }, [active.id, addLog, updateSession]);

  const handlePublish = useCallback(async () => {
    if (!publishTopic || !publishPayload) return;
    try {
      await invoke("client_publish", {
        conn: active.id,
        message: { topic: publishTopic, payload: publishPayload, qos: publishQos, retained: publishRetained },
      });
      addLog("tx", `Publish to ${publishTopic}: ${publishPayload}`);
    } catch (e) {
      addLog("error", `Publish failed: ${String(e)}`);
    }
  }, [publishTopic, publishPayload, publishQos, publishRetained, active.id, addLog]);

  // Publish to an explicit topic+value (used by the row editor in client mode).
  const handlePublishTo = useCallback(async (topic: string, payload: string) => {
    if (!topic || !clientConnected) return;
    try {
      await invoke("client_publish", {
        conn: active.id,
        message: { topic, payload, qos: publishQos, retained: publishRetained },
      });
      addLog("tx", `Publish to ${topic}: ${payload}`);
    } catch (e) {
      addLog("error", `Publish failed: ${String(e)}`);
    }
  }, [publishQos, publishRetained, clientConnected, active.id, addLog]);

  // Delete the broker's retained message on each topic by publishing an empty retained one.
  const handleClearRetained = useCallback(
    async (topics: string[]): Promise<number> => {
      const conn = active.id;
      const cleared: string[] = [];
      for (const topic of topics) {
        try {
          await invoke("client_publish", {
            conn,
            message: { topic, payload: "", qos: "atleastonce", retained: true },
          });
          cleared.push(topic);
        } catch (e) {
          addLog("error", `Clearing retained message on ${topic} failed: ${String(e)}`);
        }
      }
      updateSession(conn, (s) => markRetainedCleared(s, cleared));
      addLog("tx", `Cleared ${cleared.length} retained message${cleared.length === 1 ? "" : "s"}`);
      return cleared.length;
    },
    [active.id, addLog, updateSession],
  );

  const openClearRetained = useCallback(() => {
    if (!requirePro("Clearing retained messages")) return;
    setRetainedOpen(true);
  }, [requirePro]);

  // Install a protobuf schema, remember it, and decode what is already on screen again.
  const applyProto = useCallback(
    async (next: ProtoSettings) => {
      await invoke("proto_configure", { source: next.source, rules: next.rules });
      saveProto(next);
      setProtoSettings(next);
      addLog("info", next.source.trim() ? "Protobuf schema applied" : "Protobuf schema removed");

      // Decode stored messages again, oldest first. Results are matched back by object identity so
      // messages that arrive while the backend works are left alone.
      const redecode = async (topics: string[], infos: PayloadInfo[]): Promise<(Decoded | undefined)[]> => {
        const res = await invoke<Redecoded[]>("decode_payloads", {
          items: infos.map((info, id) => ({
            id,
            topic: topics[id],
            hex: Array.from(payloadBytes(info), (b) => b.toString(16).padStart(2, "0")).join(""),
          })),
        });
        const byId = new Map(res.map((r) => [r.id, r.decoded ?? undefined]));
        return infos.map((_, i) => byId.get(i));
      };
      for (const s of sessions) {
        const msgs = s.messages;
        const vals = Object.entries(s.topicValues);
        const out = await redecode(
          [...msgs.map((m) => m.topic), ...vals.map(([t]) => t)],
          [...msgs, ...vals.map(([, v]) => v)],
        );
        const byMsg = new Map(msgs.map((m, i) => [m, out[i]]));
        const byVal = new Map(vals.map(([, v], i) => [v, out[msgs.length + i]]));
        updateSession(s.id, (cur) => ({
          ...cur,
          messages: cur.messages.map((m) => (byMsg.has(m) ? { ...m, decoded: byMsg.get(m) } : m)),
          topicValues: Object.fromEntries(
            Object.entries(cur.topicValues).map(([t, v]) => [t, byVal.has(v) ? { ...v, decoded: byVal.get(v) } : v]),
          ),
        }));
      }
      if (brokerMessages.length > 0) {
        const out = await redecode(brokerMessages.map((m) => m.topic), brokerMessages);
        const byMsg = new Map(brokerMessages.map((m, i) => [m, out[i]]));
        setBrokerMessages((cur) => cur.map((m) => (byMsg.has(m) ? { ...m, decoded: byMsg.get(m) } : m)));
      }
    },
    [sessions, brokerMessages, addLog, updateSession],
  );

  const openDecoders = useCallback(() => {
    if (!requirePro("Sparkplug B and protobuf decoding")) return;
    setDecoderOpen(true);
  }, [requirePro]);


  // Parse an mqtt:// or mqtts:// URL into the connection form fields.
  const handleLoadConnUrl = useCallback(() => {
    const raw = connUrl.trim();
    if (!raw) return;
    try {
      const url = new URL(raw);
      const tls = url.protocol === "mqtts:" || url.protocol === "ssl:";
      const init: Record<string, string | number> = {};
      if (url.hostname) init.host = url.hostname;
      init.port = url.port ? Number(url.port) : tls ? 8883 : 1883;
      if (url.username) init.username = decodeURIComponent(url.username);
      if (url.password) init.password = decodeURIComponent(url.password);
      setParsedInit(init);
      if (tls) setClientTlsEnabled(true);
      setFormKey((k) => k + 1);
      addLog("info", `Loaded ${url.protocol}//${url.hostname}:${init.port}`);
    } catch {
      addLog("error", "Invalid MQTT URL, use mqtt://host:port or mqtts://host:port");
    }
  }, [connUrl, addLog]);

  // Save the current form fields (+ TLS) as a named profile. The free tier keeps one profile.
  const openSaveProfile = useCallback(() => {
    if (profiles.length >= FREE_PROFILE_LIMIT && !requirePro("More than one saved connection profile")) return;
    setSaveAsName(selectedProfile || String(formValues.host || "broker"));
    setSaveAsOpen(true);
  }, [profiles.length, requirePro, selectedProfile, formValues.host]);

  const handleSaveProfile = useCallback(() => {
    const name = saveAsName.trim();
    if (!name) return;
    const isNew = !profiles.some((x) => x.name === name);
    if (isNew && profiles.length >= FREE_PROFILE_LIMIT && !requirePro("More than one saved connection profile")) return;
    const p: ConnProfile = {
      name,
      host: String(formValues.host ?? ""),
      port: Number(formValues.port) || 1883,
      clientId: String(formValues.clientId ?? ""),
      username: formValues.username ? String(formValues.username) : undefined,
      password: formValues.password ? String(formValues.password) : undefined,
      tls: clientTlsEnabled,
    };
    setProfiles(upsertProfile(p));
    setSelectedProfile(p.name);
    setSaveAsOpen(false);
    addLog("info", `Saved profile "${p.name}"`);
  }, [saveAsName, profiles, requirePro, formValues, clientTlsEnabled, addLog]);

  // Load a profile into the connection form.
  const handleLoadProfile = useCallback((name: string) => {
    setSelectedProfile(name);
    const p = profiles.find((x) => x.name === name);
    if (!p) return;
    const init: Record<string, string | number> = {
      host: p.host, port: p.port, clientId: p.clientId,
    };
    if (p.username) init.username = p.username;
    if (p.password) init.password = p.password;
    setParsedInit(init);
    setFormValues(init); // mirror so Save-after-load captures the loaded values.
    setClientTlsEnabled(p.tls);
    setFormKey((k) => k + 1);
    addLog("info", `Loaded profile "${p.name}"`);
  }, [profiles, addLog]);

  const handleDeleteProfile = useCallback(() => {
    if (!selectedProfile) return;
    const name = selectedProfile;
    setProfiles(deleteProfile(name));
    setSelectedProfile("");
    addLog("info", `Deleted profile "${name}"`);
  }, [selectedProfile, addLog]);

  // ── Broker handlers ──
  const handleBrokerStart = useCallback(async () => {
    if (!requirePro("Local test broker")) return;
    try {
      const validUsers = brokerUsers.filter((u) => u.username.trim() && u.password);
      await invoke("broker_start", {
        config: {
          enabled: true,
          port: brokerPort,
          max_connections: brokerMaxConn,
          auth:
            brokerAuthEnabled && validUsers.length
              ? { mode: "password", users: validUsers }
              : { mode: "none", users: [] },
          ws_port: brokerWsPort === "" ? null : Number(brokerWsPort) || null,
          tls: {
            enabled: brokerTlsEnabled,
            cert_pem: brokerTlsCert.trim() || undefined,
            key_pem: brokerTlsKey.trim() || undefined,
            ca_pem: brokerTlsCa.trim() || undefined,
          },
        },
      });
      setBrokerRunning(true);
      addLog("info", `Broker started on port ${brokerPort}`);
    } catch (e) {
      addLog("error", `Broker start failed: ${String(e)}`);
    }
  }, [
    requirePro, brokerPort, brokerMaxConn, brokerAuthEnabled, brokerUsers, brokerWsPort,
    brokerTlsEnabled, brokerTlsCert, brokerTlsKey, brokerTlsCa, addLog,
  ]);

  const handleBrokerStop = useCallback(async () => {
    try {
      await invoke("broker_stop");
      setBrokerRunning(false);
      setBrokerTopics([]);
      setBlinkingTopics(new Set());
      lastTopicUpdateRef.current.clear();
      addLog("info", "Broker stopped");
    } catch (e) {
      addLog("error", `Broker stop failed: ${String(e)}`);
    }
  }, [addLog]);

  const handleBrokerPublish = useCallback(async (topic: string, payload: string) => {
    try {
      await invoke("broker_publish", { topic, payload });
      addLog("tx", `Broker publish to ${topic}: ${payload}`);
    } catch (e) {
      addLog("error", `Broker publish failed: ${String(e)}`);
    }
  }, [addLog]);

  const isConnected = mode === "client" ? clientConnected : brokerRunning;
  const chartConn = mode === "broker" ? "broker" : active.id;

  const fmtTs = (ts: number) =>
    new Date(ts).toLocaleTimeString("en-US", {
      hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit", fractionalSecondDigits: 3,
    });

  const handleClearData = () => {
    if (mode === "broker") {
      // Clear the per-topic table locally; the poll refills it from the broker.
      setBrokerTopics([]);
      setBlinkingTopics(new Set());
      lastTopicUpdateRef.current.clear();
    } else {
      updateSession(active.id, clearData);
    }
  };

  // Text a cell or tree leaf shows in the selected payload format.
  const fmt = useCallback((info: PayloadInfo) => formatPayload(info, payloadFormat), [payloadFormat]);

  // Pro-only parts of a payload are dropped before rendering for free users.
  const visible = (info: PayloadInfo): PayloadInfo => (pro ? info : { payload: info.payload, rawHex: info.rawHex });

  // Payload cell: formatted text, plus an inspect button when the message was decoded.
  const renderCell = useCallback((key: string, row: Record<string, unknown>) => {
    if (key === "payload") {
      const info = row._info as PayloadInfo; // every row below carries the info it was built from
      const decoded = info.decoded;
      return (
        <span style={payloadCellStyle}>
          {decoded && (
            <IconButton
              label="Inspect decoded payload"
              size="sm"
              style={{ height: 16, width: 16, marginRight: 4, verticalAlign: "middle" }}
              onClick={(e) => {
                e.stopPropagation();
                setInspecting({ topic: String(row.topic), info: { ...info, decoded } });
              }}
            >
              <Braces size={12} strokeWidth={1.75} />
            </IconButton>
          )}
          {String(row.payload ?? "")}
        </span>
      );
    }
    if (key === "retain") {
      return row.retain === "Yes" ? <Badge tone="info" mono>Yes</Badge> : "No";
    }
    return String(row[key] ?? "");
  }, []);

  const topicsColumns = [
    { key: "topic", label: "Topic", width: "230px" },
    { key: "payload", label: "Payload" },
    { key: "count", label: "Msgs", width: "60px" },
    { key: "time", label: "Last Update", width: "110px" },
    { key: "retain", label: "Retain", width: "60px" },
  ];
  const historyColumns = mode === "broker"
    ? [
        { key: "topic", label: "Topic", width: "210px" },
        { key: "payload", label: "Payload" },
        { key: "time", label: "Time", width: "110px" },
        { key: "retain", label: "Retain", width: "60px" },
      ]
    : [
        { key: "topic", label: "Topic", width: "190px" },
        { key: "payload", label: "Payload" },
        { key: "qos", label: "QoS", width: "50px" },
        { key: "time", label: "Time", width: "110px" },
        { key: "retain", label: "Retain", width: "60px" },
      ];

  const brokerTableData = brokerTopics.map((t) => {
    const info = visible(payloadInfoFromEvent(t));
    return {
      topic: t.topic,
      payload: fmt(info),
      _info: info,
      count: t.message_count,
      time: fmtTs(t.last_update),
      retain: t.retain ? "Yes" : "No",
    };
  });
  const brokerRowStyle = (row: Record<string, unknown>) =>
    blinkingTopics.has(String(row.topic)) ? { background: "var(--accent)", color: "var(--accent-fg)" } : undefined;

  // Client-mode table: latest value per subscribed topic (parity with broker).
  const clientTopicEntries = Object.entries(clientTopicValues).map(([topic, v]) => {
    const info = visible(v);
    return {
      topic,
      payload: fmt(info),
      _info: info,
      count: v.count,
      time: fmtTs(v.ts),
      retain: v.retain ? "Yes" : "No",
    };
  });
  const clientRowStyle = (row: Record<string, unknown>) =>
    blinkingClientTopics.has(String(row.topic)) ? { background: "var(--accent)", color: "var(--accent-fg)" } : undefined;

  // Topic-tree entries need real timestamps (the table renders pre-formatted time).
  const treeEntries: TreeEntry[] =
    mode === "broker"
      ? brokerTopics.map((t) => ({
          ...visible(payloadInfoFromEvent(t)), topic: t.topic, count: t.message_count,
          ts: t.last_update, retain: t.retain,
        }))
      : Object.entries(clientTopicValues).map(([topic, v]) => ({
          ...visible(v), topic, count: v.count, ts: v.ts, retain: v.retain,
        }));
  const treeBlinking = mode === "broker" ? blinkingTopics : blinkingClientTopics;

  const historyRows = (mode === "broker" ? brokerMessages : clientMessages).map((m) => {
    const info = visible(m);
    return {
      topic: m.topic,
      payload: fmt(info),
      _info: info,
      qos: "qos" in m ? m.qos : 0,
      time: fmtTs(m.ts),
      retain: m.retain ? "Yes" : "No",
    };
  });

  const dataCount = mode === "broker" ? brokerTopics.length : clientTopicEntries.length;
  const historyCount = mode === "broker" ? brokerMessages.length : clientMessages.length;
  const showClear =
    (dataView === "topics" && dataCount > 0) || (dataView === "history" && historyCount > 0);
  const hasDecoded =
    mode === "broker"
      ? brokerTopics.some((t) => t.decoded)
      : Object.values(clientTopicValues).some((v) => v.decoded);

  const retainedCandidates = retainedTopics(active, () => true);

  const handleExportHistory = async () => {
    if (!requirePro("Message history export")) return;
    const toRow = (m: BrokerMsg | ClientMsg, qos: number | null) => ({
      timestamp: m.ts,
      topic: m.topic,
      payload: m.rawHex ?? m.payload,
      encoding: m.rawHex ? ("hex" as const) : ("utf8" as const),
      qos,
      retain: m.retain,
      decoded: m.decoded ? JSON.stringify(m.decoded.error !== undefined ? { error: m.decoded.error } : m.decoded.value) : "",
    });
    const csv = messagesToCsv(
      mode === "broker" ? brokerMessages.map((m) => toRow(m, null)) : clientMessages.map((m) => toRow(m, m.qos)),
    );
    try {
      const stamp = new Date().toISOString().slice(0, 19).replace(/:/g, "-");
      const path = await saveTextFile(`mqtt-messages-${stamp}.csv`, csv);
      if (path) addLog("info", `Message history exported to ${path}`);
    } catch (e) {
      addLog("error", `Export failed: ${String(e)}`);
    }
  };

  const dataTable = (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
      <Tabs
        label="Data view"
        value={dataView}
        onChange={(v) => {
          if (v === "charts" && !requirePro("Numeric topic charts")) return;
          setDataView(v);
        }}
        items={[
          { value: "topics", label: "Topics", count: dataCount },
          { value: "tree", label: "Tree", count: dataCount },
          { value: "history", label: "History", count: historyCount },
          { value: "charts", label: <>Charts {!pro && <ProBadge />}</> },
        ]}
        trailing={
          <>
            <SheetSelect
              size="sm"
              sheetTitle="Payload format"
              style={{ width: 110 }}
              value={payloadFormat}
              options={[
                { value: "raw", label: "Raw" },
                { value: "json", label: "JSON" },
                { value: "hex", label: "Hex" },
                { value: "base64", label: "Base64" },
                { value: "decoded", label: pro ? "Decoded" : "Decoded (Pro)" },
              ] satisfies SheetOption<PayloadFormat>[]}
              onChange={(v) => {
                if (v === "decoded" && !requirePro("Sparkplug B and protobuf decoding")) return;
                setPayloadFormat(v);
              }}
              aria-label="Payload format"
              title="Payload format"
            />
            <Button size="sm" variant="ghost" onClick={openDecoders} title="Sparkplug B and protobuf decoders">
              Decoders {!pro && <ProBadge />}
            </Button>
            {dataView === "history" && historyCount > 0 && (
              <Button size="sm" variant="ghost" onClick={handleExportHistory}>
                Export {!pro && <ProBadge />}
              </Button>
            )}
            {showClear && (
              <Button size="sm" variant="ghost" onClick={handleClearData}>
                Clear
              </Button>
            )}
          </>
        }
      />

      <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", padding: "4px 2px 6px" }}>
        {dataView === "topics"
          ? `Latest value per topic, ${dataCount} topic${dataCount !== 1 ? "s" : ""}`
          : dataView === "tree"
            ? `Hierarchical topic tree, ${dataCount} topic${dataCount !== 1 ? "s" : ""}`
            : dataView === "history"
              ? `Message stream, ${historyCount} message${historyCount !== 1 ? "s" : ""}`
              : "Numeric values over time"}
        {hasDecoded && dataView !== "charts" && payloadFormat !== "decoded" &&
          (pro
            ? ". Sparkplug B or protobuf payloads found, choose Decoded to read them"
            : ". Sparkplug B or protobuf payloads found, decoding is a Pro feature")}
      </span>

      {dataView === "charts" ? (
        <ChartPanel
          store={chartStore}
          conn={chartConn}
          selected={chartSelected[chartConn] ?? []}
          onSelectedChange={(keys) => setChartSelected((prev) => ({ ...prev, [chartConn]: keys }))}
        />
      ) : dataView === "tree" ? (
        treeEntries.length === 0 ? (
          <EmptyState
            compact
            icon={<RadioTower size={18} strokeWidth={1.75} />}
            title={mode === "broker" ? "No topics yet" : "No messages yet"}
            description={
              mode === "broker"
                ? "Publish to the broker (or connect a client) to build the topic tree."
                : "Subscribe to a topic to build the topic tree."
            }
          />
        ) : (
          <TopicTree
            entries={treeEntries}
            formatPayload={fmt}
            onLeafClick={(t) => setEditingTopic(t)}
            blinkingTopics={treeBlinking}
          />
        )
      ) : dataView === "topics" ? (
        mode === "broker" ? (
          <DataTable
            columns={topicsColumns}
            data={brokerTableData}
            selectable
            renderCell={renderCell}
            onRowClick={(row) => setEditingTopic(String(row.topic))}
            getRowStyle={brokerRowStyle}
            emptyMessage="No topics yet. Publish to the broker to see values here."
          />
        ) : (
          <DataTable
            columns={topicsColumns}
            data={clientTopicEntries}
            selectable
            renderCell={renderCell}
            onRowClick={(row) => setEditingTopic(String(row.topic))}
            getRowStyle={clientRowStyle}
            emptyMessage="No messages yet. Subscribe to a topic to see values here."
          />
        )
      ) : (
        <DataTable
          columns={historyColumns}
          data={historyRows}
          autoScroll
          renderCell={renderCell}
          emptyMessage={mode === "broker" ? "No messages yet. Publish to see the stream." : "No messages yet. Subscribe to see the stream."}
        />
      )}
    </div>
  );

  // ── Sidebar content ──
  const sidebarContent = (
    <>
      <div style={{ padding: "12px 12px 0" }}>
        <Segmented
          label="Mode"
          block
          value={mode}
          onChange={(v) => { if (v === "broker" && !requirePro("Local test broker")) return; setMode(v); }}
          options={[
            { value: "client", label: "Client" },
            { value: "broker", label: <>Broker <ProBadge /></> },
          ]}
        />
      </div>

      {mode === "client" ? (
        <>
        <Section title="Connection">
          <div
            role="tablist"
            aria-label="Connections"
            style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 4, marginBottom: 8 }}
          >
            {sessions.map((s) => {
              const isActive = s.id === active.id;
              const v = isActive && Object.keys(formValues).length ? formValues : s.form.values;
              const label = s.status === "idle" ? `${v.host}:${v.port}` : s.label;
              const dot =
                s.status === "connected" ? "var(--success, #22c55e)"
                : s.status === "connecting" ? "var(--warning, #f59e0b)"
                : "var(--text-muted)";
              return (
                <span
                  key={s.id}
                  role="tab"
                  aria-selected={isActive}
                  tabIndex={0}
                  className="vt-badge vt-badge--mono"
                  style={{
                    gap: 4,
                    cursor: "pointer",
                    outline: isActive ? "1px solid var(--accent)" : undefined,
                    maxWidth: "100%",
                  }}
                  title={`${label}, ${s.status}`}
                  onClick={() => selectSession(s.id)}
                  onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") selectSession(s.id); }}
                >
                  <span aria-hidden="true" style={{ width: 6, height: 6, borderRadius: "50%", background: dot, flexShrink: 0 }} />
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{label}</span>
                  {sessions.length > 1 && (
                    <IconButton
                      label={`Close connection ${label}`}
                      size="sm"
                      onClick={(e) => { e.stopPropagation(); void closeSession(s.id); }}
                      style={{ height: 14, width: 14 }}
                    >
                      <X size={10} strokeWidth={1.75} />
                    </IconButton>
                  )}
                </span>
              );
            })}
            <IconButton label="Add another connection" size="sm" outline onClick={addSession}>
              <Plus size={14} strokeWidth={1.75} />
            </IconButton>
            {!pro && <ProBadge />}
          </div>
          <div style={{ display: "flex", gap: 4, marginBottom: 8 }}>
            <SheetSelect
              size="sm"
              sheetTitle="Saved profiles"
              style={{ flex: 1, minWidth: 0 }}
              value={selectedProfile}
              options={profiles.map((p) => ({ value: p.name, label: p.name }))}
              placeholder="Saved profiles…"
              onChange={(v) => handleLoadProfile(v)}
              disabled={clientConnected}
              title="Load a saved connection profile"
              aria-label="Saved profiles"
            />
            <IconButton
              label="Save current fields as a profile"
              size="sm"
              outline
              disabled={clientConnected}
              onClick={openSaveProfile}
            >
              <Plus size={16} strokeWidth={1.75} />
            </IconButton>
            <IconButton
              label="Delete selected profile"
              size="sm"
              outline
              danger
              disabled={clientConnected || !selectedProfile}
              onClick={handleDeleteProfile}
            >
              <Trash2 size={16} strokeWidth={1.75} />
            </IconButton>
          </div>
          <div style={{ display: "flex", gap: 4, marginBottom: 8 }}>
            <input
              className="vt-input vt-input--sm vt-input--mono"
              style={{ flex: 1, minWidth: 0 }}
              placeholder="mqtt://user:pass@host:1883"
              value={connUrl}
              onChange={(e) => setConnUrl(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleLoadConnUrl(); }}
              disabled={clientConnected}
              aria-label="Connection URL"
            />
            <Button size="sm" onClick={handleLoadConnUrl} disabled={clientConnected}>
              Load
            </Button>
          </div>
          <ConnectionForm
              key={formKey}
              fields={[
                { name: "host", label: "Host", type: "text", default: "127.0.0.1" },
                { name: "port", label: "Port", type: "number", default: 1883 },
                { name: "clientId", label: "Client ID", type: "text", default: "mactools-explorer" },
                { name: "username", label: "Username", type: "text", default: "" },
                { name: "password", label: "Password", type: "password", default: "" },
              ]}
              initialValues={parsedInit}
              onSubmit={handleClientConnect}
              onDisconnect={handleClientDisconnect}
              onChange={setFormValues}
              connected={clientConnected}
              loading={clientLoading}
            />
        </Section>

        <Section title="Advanced">
          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            className="vt-btn vt-btn--ghost vt-btn--sm"
            style={{ width: "100%", justifyContent: "space-between", padding: "0 4px" }}
            aria-expanded={showAdvanced}
          >
            <span>TLS, LWT, Keep-alive</span>
            {showAdvanced
              ? <ChevronDown size={14} strokeWidth={1.75} />
              : <ChevronRight size={14} strokeWidth={1.75} />}
          </button>
          {showAdvanced && (
            <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 8 }}>
              <label className="vt-check">
                <input
                  type="checkbox"
                  checked={clientTlsEnabled}
                  onChange={(e) => setClientTlsEnabled(e.target.checked)}
                />
                Use TLS (MQTTS, typically port 8883)
              </label>
              <Field label="Keep-alive (seconds)">
                <input
                  className="vt-input vt-input--sm"
                  type="number"
                  value={keepAlive}
                  onChange={(e) => setKeepAlive(Number(e.target.value))}
                />
              </Field>
              <button
                type="button"
                onClick={() => { if (showCerts || requirePro("Custom CA and client certificates (mTLS)")) setShowCerts((v) => !v); }}
                className="vt-btn vt-btn--ghost vt-btn--sm"
                style={{ width: "100%", justifyContent: "space-between", padding: "0 4px" }}
                aria-expanded={showCerts}
              >
                <span>Custom CA, client certificate <ProBadge /></span>
                {showCerts ? <ChevronDown size={14} strokeWidth={1.75} /> : <ChevronRight size={14} strokeWidth={1.75} />}
              </button>
              {showCerts && (
                <>
                  <Field label="CA Certificate (PEM), optional for a private CA">
                    <textarea
                      className="vt-textarea vt-textarea--mono"
                      style={{ minHeight: 40, resize: "vertical" }}
                      placeholder={"-----BEGIN CERTIFICATE-----\n…"}
                      value={clientCa}
                      onChange={(e) => setClientCa(e.target.value)}
                    />
                  </Field>
                  <Field label="Client Certificate (PEM), optional mTLS">
                    <textarea
                      className="vt-textarea vt-textarea--mono"
                      style={{ minHeight: 40, resize: "vertical" }}
                      placeholder={"-----BEGIN CERTIFICATE-----\n…"}
                      value={clientCert}
                      onChange={(e) => setClientCert(e.target.value)}
                    />
                  </Field>
                  <Field label="Client Key (PEM), optional mTLS">
                    <textarea
                      className="vt-textarea vt-textarea--mono"
                      style={{ minHeight: 40, resize: "vertical" }}
                      placeholder={"-----BEGIN PRIVATE KEY-----\n…"}
                      value={clientKey}
                      onChange={(e) => setClientKey(e.target.value)}
                    />
                  </Field>
                </>
              )}
              <button
                type="button"
                onClick={() => { if (showWill || requirePro("Last Will and Testament")) setShowWill((v) => !v); }}
                className="vt-btn vt-btn--ghost vt-btn--sm"
                style={{ width: "100%", justifyContent: "space-between", padding: "0 4px" }}
                aria-expanded={showWill}
              >
                <span>Last Will and Testament <ProBadge /></span>
                {showWill ? <ChevronDown size={14} strokeWidth={1.75} /> : <ChevronRight size={14} strokeWidth={1.75} />}
              </button>
              {showWill && (
                <>
              <Field label="Will topic and payload">
                <input
                  className="vt-input vt-input--sm vt-input--mono"
                  placeholder="will/topic"
                  value={lwtTopic}
                  onChange={(e) => setLwtTopic(e.target.value)}
                />
                <textarea
                  className="vt-textarea vt-textarea--mono"
                  style={{ minHeight: 40, resize: "vertical", marginTop: 6 }}
                  placeholder='{"state": "offline"}'
                  value={lwtPayload}
                  onChange={(e) => setLwtPayload(e.target.value)}
                  aria-label="Will payload"
                />
              </Field>
              <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                <div style={{ flex: 1 }}>
                  <Field label="Will QoS">
                    <SheetSelect size="sm" sheetTitle="Will QoS" value={lwtQos} options={QOS_OPTIONS} onChange={(v) => setLwtQos(v)} />
                  </Field>
                </div>
                <label className="vt-check" style={{ marginTop: 18 }}>
                  <input
                    type="checkbox"
                    checked={lwtRetain}
                    onChange={(e) => setLwtRetain(e.target.checked)}
                  />
                  Retain
                </label>
              </div>
                </>
              )}
            </div>
          )}
        </Section>
        </>
      ) : (
        <Section title="Broker server">
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <Field label="TCP port">
                <input
                  className="vt-input vt-input--sm"
                  type="number"
                  value={brokerPort}
                  onChange={(e) => setBrokerPort(Number(e.target.value))}
                  disabled={brokerRunning}
                />
              </Field>
              <Field label="Max connections">
                <input
                  className="vt-input vt-input--sm"
                  type="number"
                  value={brokerMaxConn}
                  onChange={(e) => setBrokerMaxConn(Number(e.target.value))}
                  disabled={brokerRunning}
                />
              </Field>
              <Field label="WebSocket port (optional)">
                <input
                  className="vt-input vt-input--sm"
                  type="number"
                  placeholder="e.g. 9001"
                  value={brokerWsPort}
                  onChange={(e) =>
                    setBrokerWsPort(e.target.value === "" ? "" : Number(e.target.value))
                  }
                  disabled={brokerRunning}
                />
              </Field>
              <label className="vt-check">
                <input
                  type="checkbox"
                  checked={brokerTlsEnabled}
                  onChange={(e) => setBrokerTlsEnabled(e.target.checked)}
                  disabled={brokerRunning}
                />
                Use TLS (MQTTS, typically port 8883)
              </label>
              {brokerTlsEnabled && !brokerRunning && (
                <>
                  <Field label="Server certificate (PEM)">
                    <textarea
                      className="vt-textarea vt-textarea--mono"
                      style={{ minHeight: 40, resize: "vertical" }}
                      placeholder={"-----BEGIN CERTIFICATE-----\n…"}
                      value={brokerTlsCert}
                      onChange={(e) => setBrokerTlsCert(e.target.value)}
                    />
                  </Field>
                  <Field label="Server key (PEM)">
                    <textarea
                      className="vt-textarea vt-textarea--mono"
                      style={{ minHeight: 40, resize: "vertical" }}
                      placeholder={"-----BEGIN PRIVATE KEY-----\n…"}
                      value={brokerTlsKey}
                      onChange={(e) => setBrokerTlsKey(e.target.value)}
                    />
                  </Field>
                  <Field label="CA certificate (PEM), optional mTLS">
                    <textarea
                      className="vt-textarea vt-textarea--mono"
                      style={{ minHeight: 40, resize: "vertical" }}
                      placeholder={"-----BEGIN CERTIFICATE-----\n…"}
                      value={brokerTlsCa}
                      onChange={(e) => setBrokerTlsCa(e.target.value)}
                    />
                  </Field>
                </>
              )}
              <label className="vt-check">
                <input
                  type="checkbox"
                  checked={brokerAuthEnabled}
                  onChange={(e) => setBrokerAuthEnabled(e.target.checked)}
                  disabled={brokerRunning}
                />
                Require username / password
              </label>
              {brokerAuthEnabled && !brokerRunning && (
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {brokerUsers.map((u, i) => (
                    <div key={i} style={{ display: "flex", gap: 4 }}>
                      <input
                        className="vt-input vt-input--sm"
                        style={{ flex: 1, minWidth: 0 }}
                        placeholder="username"
                        value={u.username}
                        onChange={(e) => {
                          const next = [...brokerUsers];
                          next[i] = { ...next[i], username: e.target.value };
                          setBrokerUsers(next);
                        }}
                        aria-label="Username"
                      />
                      <input
                        className="vt-input vt-input--sm"
                        style={{ flex: 1, minWidth: 0 }}
                        type="password"
                        placeholder="password"
                        value={u.password}
                        onChange={(e) => {
                          const next = [...brokerUsers];
                          next[i] = { ...next[i], password: e.target.value };
                          setBrokerUsers(next);
                        }}
                        aria-label="Password"
                      />
                      <IconButton
                        label="Remove user"
                        size="sm"
                        danger
                        onClick={() => setBrokerUsers(brokerUsers.filter((_, j) => j !== i))}
                      >
                        <Trash2 size={16} strokeWidth={1.75} />
                      </IconButton>
                    </div>
                  ))}
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setBrokerUsers([...brokerUsers, { username: "", password: "" }])}
                    icon={<Plus size={14} strokeWidth={1.75} />}
                    style={{ alignSelf: "flex-start" }}
                  >
                    Add user
                  </Button>
                </div>
              )}
            </div>
        </Section>
      )}

      {mode === "broker" && (
        <div style={{ padding: "12px" }}>
          {!brokerRunning ? (
            <Button variant="primary" block onClick={handleBrokerStart}>Start broker</Button>
          ) : (
            <Button variant="danger" block onClick={handleBrokerStop}>Stop broker</Button>
          )}
        </div>
      )}

      {mode === "broker" && brokerRunning && (
        <Section title="Publish">
          <BrokerPublishForm onPublish={handleBrokerPublish} />
        </Section>
      )}
    </>
  );

  // ── Right panel content ──
  const rightPanelContent = mode === "client" ? (
    <>
      <Section title="Subscriptions">
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ display: "flex", gap: 4 }}>
            <input
              className="vt-input vt-input--sm vt-input--mono"
              style={{ flex: 1, minWidth: 0 }}
              placeholder="topic/filter/#"
              value={subscribeTopic}
              onChange={(e) => setSubscribeTopic(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleSubscribe(); }}
              aria-label="Topic filter"
            />
            <SheetSelect
              size="sm"
              sheetTitle="Subscribe QoS"
              style={{ width: 84, flexShrink: 0 }}
              value={subscribeQos}
              options={QOS_OPTIONS}
              onChange={(v) => setSubscribeQos(v)}
              aria-label="Subscribe QoS"
            />
          </div>
          <Button variant="primary" block size="sm" onClick={handleSubscribe}>Subscribe</Button>
          <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.4 }}>
            Wildcards: <span style={monoFont}>+</span> single, <span style={monoFont}>#</span> multi-level
          </div>
          {subscriptions.length > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 4, marginTop: 4 }}>
              {subscriptions.map((s) => (
                <span key={s.topic} className="vt-badge vt-badge--mono" style={{ gap: 2 }}>
                  {s.topic}
                  <IconButton label={`Unsubscribe from ${s.topic}`} size="sm" onClick={() => handleUnsubscribe(s.topic)} style={{ height: 14, width: 14 }}>
                    <X size={10} strokeWidth={1.75} />
                  </IconButton>
                </span>
              ))}
            </div>
          )}
          <Button size="sm" variant="ghost" block disabled={!clientConnected} onClick={openClearRetained}>
            Clear retained messages… {!pro && <ProBadge />}
          </Button>
        </div>
      </Section>

      <Section title="Publish">
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <Field label="Topic">
            <input
              className="vt-input vt-input--sm vt-input--mono"
              placeholder="topic/to/publish"
              value={publishTopic}
              onChange={(e) => setPublishTopic(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handlePublish(); }}
            />
          </Field>
          <Field label="Payload">
            <textarea
              className="vt-textarea vt-textarea--mono"
              style={{ minHeight: 48, resize: "vertical" }}
              placeholder='{"key": "value"}'
              value={publishPayload}
              onChange={(e) => setPublishPayload(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && e.metaKey) handlePublish(); }}
            />
          </Field>
          <div style={{ display: "flex", gap: 8, alignItems: "flex-end" }}>
            <div style={{ flex: 1 }}>
              <Field label="QoS">
                <SheetSelect size="sm" sheetTitle="QoS" value={publishQos} options={QOS_OPTIONS} onChange={(v) => setPublishQos(v)} />
              </Field>
            </div>
            <label className="vt-check" style={{ paddingBottom: 7 }}>
              <input
                type="checkbox"
                checked={publishRetained}
                onChange={(e) => setPublishRetained(e.target.checked)}
              />
              Retain
            </label>
          </div>
          <Button variant="primary" block size="sm" onClick={handlePublish}>Publish</Button>
        </div>
      </Section>
    </>
  ) : brokerRunning && brokerTopics.length > 0 ? (
    <Section title="Topics" meta={`${brokerTopics.length}`}>
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        {brokerTopics.slice(0, 30).map((t) => (
          <div
            key={t.topic}
            style={{
              display: "flex", justifyContent: "space-between", padding: "2px 6px", fontSize: "var(--fs-xs)",
              borderRadius: "var(--radius-sm)",
              background: blinkingTopics.has(t.topic) ? "var(--accent)" : "transparent",
              color: blinkingTopics.has(t.topic) ? "var(--accent-fg)" : undefined,
              transition: "background 0.15s",
            }}
          >
            <span style={{ ...monoFont, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{t.topic}</span>
            <span style={{ color: "inherit", flexShrink: 0, marginLeft: 8, fontVariantNumeric: "tabular-nums" }}>
              {t.message_count}
            </span>
          </div>
        ))}
      </div>
    </Section>
  ) : null;

  // ── Mobile tab bar ──
  const mobileTabBar = (
    <nav style={{
      padding: "6px 12px",
      paddingBottom: "calc(6px + env(safe-area-inset-bottom, 0px))",
      background: "var(--card)",
      borderTop: "1px solid var(--border)",
      flexShrink: 0,
    }}>
      <div style={{
        display: "flex",
        width: "100%",
        borderRadius: 8,
        background: "var(--bg)",
        border: "1px solid var(--border-light)",
        padding: 3,
        gap: 2,
      }}>
        {([
          { id: "connect" as const, label: "Connect" },
          { id: "data" as const, label: "Data" },
          { id: "topics" as const, label: "Topics" },
          { id: "log" as const, label: "Log" },
        ] as const).map((tab) => (
          <button
            key={tab.id}
            onClick={() => setMobileTab(tab.id)}
            style={{
              flex: 1,
              padding: "6px 0",
              fontSize: "var(--fs-xs)",
              fontWeight: mobileTab === tab.id ? 700 : 500,
              background: mobileTab === tab.id ? "var(--border)" : "transparent",
              color: mobileTab === tab.id ? "var(--text)" : "var(--text-muted)",
              border: "none",
              borderRadius: 6,
              cursor: "pointer",
              transition: "color 0.15s, background 0.15s",
              textAlign: "center",
            }}
          >
            {tab.label}
          </button>
        ))}
      </div>
    </nav>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar title="MQTT Explorer" icon={appIcon} connected={isConnected} onDisconnect={mode === "client" ? handleClientDisconnect : handleBrokerStop} onInfo={() => setShowInfo(true)} />

      {isMobile ? (
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
            {mobileTab === "connect" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                {sidebarContent}
              </div>
            )}
            {mobileTab === "data" && dataTable}
            {mobileTab === "topics" &&
              (mode === "broker" ? (
                <DataTable columns={topicsColumns} data={brokerTableData} selectable renderCell={renderCell} getRowStyle={brokerRowStyle} onRowClick={(row) => setEditingTopic(String(row.topic))} />
              ) : (
                <DataTable columns={topicsColumns} data={clientTopicEntries} selectable renderCell={renderCell} getRowStyle={clientRowStyle} onRowClick={(row) => setEditingTopic(String(row.topic))} />
              ))}
            {mobileTab === "log" && <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />}
          </div>
          {mobileTabBar}
          <StatusBar connected={isConnected} device={mode === "client" ? clientEndpoint : brokerRunning ? { ip: "localhost", port: brokerPort } : null} latency={mode === "client" ? clientRtt ?? undefined : undefined} error={clientError} />
        </div>
      ) : (
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside className="vt-sidebar" style={{ ...sidebarStyle, width: leftWidth, cursor: dragSide ? "col-resize" : undefined }}>
            {sidebarContent}
          </aside>
          <div
            onMouseDown={(e) => handleDragStart("left", e)}
            style={{ width: 4, cursor: "col-resize", background: dragSide === "left" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
            onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
            onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
          />
          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", padding: 8, gap: 0 }}>
            <div style={{ height: tableHeight, minHeight: 120, overflow: "hidden", flexShrink: 0, display: "flex", flexDirection: "column" }}>
              {dataTable}
            </div>
            <div
              onMouseDown={(e) => handleDragStart("table", e)}
              style={{ height: 4, cursor: "row-resize", background: dragSide === "table" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
              onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
              onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
            />
            <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
              <LogPanel entries={log} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
            </div>
          </main>

          {rightPanelContent && (
            <>
              <div
                onMouseDown={(e) => handleDragStart("right", e)}
                style={{ width: 4, cursor: "col-resize", background: dragSide === "right" ? "var(--accent)" : "transparent", flexShrink: 0, zIndex: 10, transition: "background 0.15s" }}
                onMouseEnter={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "var(--border)"; }}
                onMouseLeave={(e) => { if (!dragSide) (e.target as HTMLElement).style.background = "transparent"; }}
              />
              <aside className="vt-sidebar" style={{
                width: rightWidth,
                flexShrink: 0,
                borderLeft: "1px solid var(--border)",
                borderRight: "none",
                cursor: dragSide ? "col-resize" : undefined,
              }}>
                {rightPanelContent}
              </aside>
            </>
          )}
        </div>
      )}

      {!isMobile && <StatusBar connected={isConnected} device={mode === "client" ? clientEndpoint : brokerRunning ? { ip: "localhost", port: brokerPort } : null} latency={mode === "client" ? clientRtt ?? undefined : undefined} error={clientError} />}
      {editingTopic && (
        <ValueEditor
          address={editingTopic}
          onSubmit={(value, _dataType) => {
            if (mode === "broker") {
              handleBrokerPublish(editingTopic, value);
            } else {
              handlePublishTo(editingTopic, value);
            }
            setEditingTopic(null);
          }}
          onCancel={() => setEditingTopic(null)}
        />
      )}
      <Dialog
        open={saveAsOpen}
        onClose={() => setSaveAsOpen(false)}
        title="Save connection profile"
        width={340}
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setSaveAsOpen(false)}>Cancel</Button>
            <Button size="sm" variant="primary" disabled={!saveAsName.trim()} onClick={handleSaveProfile}>Save</Button>
          </>
        }
      >
        <Field label="Profile name">
          <input
            className="vt-input vt-input--sm"
            value={saveAsName}
            autoFocus
            onChange={(e) => setSaveAsName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") handleSaveProfile(); }}
          />
        </Field>
        <p style={{ margin: "8px 0 0", fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.4 }}>
          Saved on this Mac, including the password if one is filled in.
        </p>
      </Dialog>
      <Dialog
        open={showInfo}
        onClose={() => setShowInfo(false)}
        title="MQTT Explorer"
        width={360}
        footer={
          <>
            {/* No outbound source link in App Store builds: review treats links to builds that unlock paid
                features elsewhere as steering away from in-app purchase. */}
            {proStatus && !proStatus.store && (
              <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
                Source on GitHub
              </Button>
            )}
            <Button onClick={() => setShowInfo(false)}>Close</Button>
          </>
        }
      >
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.6 }}>
          <p style={{ margin: "0 0 10px" }}>MQTT client, topic inspector, and local test broker in one app.</p>
          <p style={{ margin: "0 0 10px" }}>
            Connect to any MQTT broker to subscribe, publish, and watch live message streams. Pro adds an embedded
            broker to test against, several connections at once, Sparkplug B and protobuf decoding, and charts of
            numeric values.
          </p>
          <p style={{ margin: "0 0 12px", color: "var(--text-muted)" }}>
            Part of the <strong style={{ color: "var(--text)" }}>Voltrus</strong> suite by JoyoDigitama
          </p>
          <ProAboutRow />
        </div>
      </Dialog>
      {inspecting && (
        <PayloadInspector topic={inspecting.topic} info={inspecting.info} onClose={() => setInspecting(null)} />
      )}
      <ClearRetainedDialog
        open={retainedOpen}
        onClose={() => setRetainedOpen(false)}
        candidates={retainedCandidates}
        defaultFilter={subscriptions.length === 1 ? subscriptions[0].topic : "#"}
        onClear={handleClearRetained}
      />
      <DecoderDialog
        open={decoderOpen}
        onClose={() => setDecoderOpen(false)}
        settings={protoSettings}
        onApply={applyProto}
      />
    </div>
  );
}

// ── Broker Publish Form subcomponent ──
function BrokerPublishForm({ onPublish }: { onPublish: (topic: string, payload: string) => void }) {
  const [topic, setTopic] = useState("");
  const [payload, setPayload] = useState("");
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <Field label="Topic">
        <input
          className="vt-input vt-input--sm vt-input--mono"
          placeholder="topic/to/publish"
          value={topic}
          onChange={(e) => setTopic(e.target.value)}
        />
      </Field>
      <Field label="Payload">
        <textarea
          className="vt-textarea vt-textarea--mono"
          style={{ minHeight: 48, resize: "vertical" }}
          placeholder='{"key": "value"}'
          value={payload}
          onChange={(e) => setPayload(e.target.value)}
        />
      </Field>
      <Button
        variant="primary"
        block
        size="sm"
        disabled={!topic || !payload}
        onClick={() => { onPublish(topic, payload); }}
      >
        Publish
      </Button>
    </div>
  );
}

export default App;
