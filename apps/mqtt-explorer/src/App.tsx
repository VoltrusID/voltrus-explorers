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
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import {
  Trash2,
  Plus,
  X,
  ChevronDown,
  ChevronRight,
  RadioTower,
} from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";
import { TopicTree, type TreeEntry } from "./TopicTree";
import { loadProfiles, upsertProfile, deleteProfile, FREE_PROFILE_LIMIT, type ConnProfile } from "./profiles";
import { messagesToCsv } from "./csv";

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

interface TopicInfo {
  topic: string;
  last_payload: string;
  last_update: number;
  message_count: number;
  retain: boolean;
}

interface Subscription {
  topic: string;
  qos: string;
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

/** Proper MQTT topic matching with wildcard support. */
function mqttTopicMatch(subscription: string, topic: string): boolean {
  const subLevels = subscription.split("/");
  const topicLevels = topic.split("/");

  for (let i = 0; i < subLevels.length; i++) {
    const subLevel = subLevels[i];

    // Multi-level wildcard: matches all remaining levels
    if (subLevel === "#") {
      return true;
    }

    // Topic has fewer levels than subscription (and no wildcard)
    if (i >= topicLevels.length) {
      return false;
    }

    // Single-level wildcard: matches exactly one level
    if (subLevel === "+") {
      continue;
    }

    // Literal comparison
    if (subLevel !== topicLevels[i]) {
      return false;
    }
  }

  // All subscription levels matched; topic must have the same number of levels
  return subLevels.length === topicLevels.length;
}

function App() {
  const { requirePro, pro, status: proStatus } = useProStatus();
  const [mode, setMode] = useState<"client" | "broker">("client");
  const [log, setLog] = useState<LogEntry[]>([]);
  const [editingTopic, setEditingTopic] = useState<string | null>(null);
  const [showInfo, setShowInfo] = useState(false);

  // Client state
  const [clientConnected, setClientConnected] = useState(false);
  const [clientLoading, setClientLoading] = useState(false);
  const [clientError, setClientError] = useState<string | null>(null);
  const [subscribeTopic, setSubscribeTopic] = useState("");
  const [subscriptions, setSubscriptions] = useState<Subscription[]>([]);
  const [publishTopic, setPublishTopic] = useState("");
  const [publishPayload, setPublishPayload] = useState("");
  const [clientMessages, setClientMessages] = useState<any[]>([]);
  // Latest value per topic for the client-mode table (parity with broker mode).
  const [clientTopicValues, setClientTopicValues] = useState<
    Record<string, { payload: string; ts: number; retain: boolean; count: number }>
  >({});
  const [blinkingClientTopics, setBlinkingClientTopics] = useState<Set<string>>(new Set());
  const [subscribeQos, setSubscribeQos] = useState<string>("atmostonce");
  const [publishQos, setPublishQos] = useState<string>("atmostonce");
  const [publishRetained, setPublishRetained] = useState(false);

  // Data panel view + payload rendering.
  const [dataView, setDataView] = useState<"topics" | "tree" | "history">("topics");
  const [payloadFormat, setPayloadFormat] = useState<"raw" | "json" | "hex" | "base64">("raw");

  // Connection-URL paste + endpoint shown in the status bar.
  const [connUrl, setConnUrl] = useState("");
  const [formKey, setFormKey] = useState(0);
  const [parsedInit, setParsedInit] = useState<Record<string, string | number> | undefined>(undefined);
  const [clientEndpoint, setClientEndpoint] = useState<{ ip: string; port: number } | null>(null);
  // Live PING round-trip latency (ms) reported by the backend, client mode.
  const [clientRtt, setClientRtt] = useState<number | null>(null);

  // Saved connection profiles (persisted) + the live form-field snapshot.
  const [profiles, setProfiles] = useState<ConnProfile[]>([]);
  const [selectedProfile, setSelectedProfile] = useState("");
  const [formValues, setFormValues] = useState<Record<string, string | number>>({});

  // Broker message history (broker mode already had per-topic; this adds a stream).
  const [brokerMessages, setBrokerMessages] = useState<any[]>([]);

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

  // Blink a client topic briefly when its value changes.
  const blinkClientTopic = useCallback((topic: string) => {
    setBlinkingClientTopics((cur) => {
      if (cur.has(topic)) return cur;
      const next = new Set(cur); next.add(topic); return next;
    });
    window.setTimeout(() => {
      setBlinkingClientTopics((cur) => {
        if (!cur.has(topic)) return cur;
        const next = new Set(cur); next.delete(topic); return next;
      });
    }, 600);
  }, []);

  // Single ingest path for every received client message: append to history,
  // update latest-value-per-topic with a per-topic count, and blink on change.
  const ingestClientMessage = useCallback(
    (m: { topic?: string; payload?: string; retain?: boolean; qos?: number; _ts?: number; timestamp?: number }) => {
      const topic = String(m.topic ?? "");
      if (!topic) return;
      const ts = m._ts ?? m.timestamp ?? Date.now();
      const payload = String(m.payload ?? "");
      const retain = !!m.retain;
      setClientMessages((prev) => [...prev.slice(-499), { ...m, topic, payload, retain, qos: m.qos ?? 0, _ts: ts }]);
      setClientTopicValues((prev) => {
        const ex = prev[topic];
        const isNewValue = !ex || ts > ex.ts;
        const next = { ...prev, [topic]: { payload, ts, retain, count: (ex?.count ?? 0) + 1 } };
        if (isNewValue) blinkClientTopic(topic);
        return next;
      });
    },
    [blinkClientTopic],
  );

  // Event: listen for client messages and disconnects.
  useEffect(() => {
    let unlistenClient: UnlistenFn | null = null;
    let unlistenDisconnect: UnlistenFn | null = null;
    let unlistenRtt: UnlistenFn | null = null;

    listen<{ topic: string; payload: string; qos: number; retain: boolean }>("client-message", (event) => {
      ingestClientMessage({ ...event.payload, _ts: Date.now() });
    }).then((fn) => { unlistenClient = fn; });

    listen<{ rtt: number }>("client-rtt", (event) => {
      setClientRtt(event.payload.rtt);
    }).then((fn) => { unlistenRtt = fn; });

    listen<{ reason: string; detail?: string }>("client-disconnected", (event) => {
      setClientConnected(false);
      setClientRtt(null);
      setClientEndpoint(null);
      setSubscriptions([]);
      setClientError(`Connection lost: ${event.payload.detail ?? event.payload.reason}`);
    }).then((fn) => { unlistenDisconnect = fn; });

    return () => {
      unlistenClient?.();
      unlistenDisconnect?.();
      unlistenRtt?.();
    };
  }, [ingestClientMessage]);

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
    listen<{ topic: string; payload: string; timestamp: number; retain: boolean }>("broker-message", (event) => {
      const ts = event.payload.timestamp ?? Date.now();
      setBrokerMessages((prev) => [...prev.slice(-499), { ...event.payload, _ts: ts }]);
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);



  // ── Client handlers ──
  const handleClientConnect = useCallback(
    async (values: Record<string, string | number>) => {
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
      setClientLoading(true);
      setClientError(null);
      // Fresh session — clear any data from a previous connection.
      setClientMessages([]);
      setClientTopicValues({});
      setBlinkingClientTopics(new Set());
      try {
        await invoke("client_connect", { config });
        setClientConnected(true);
        setClientEndpoint({ ip: config.host, port: config.port });
        addLog("info", `Connected to ${config.host}:${config.port}`);
      } catch (e) {
        setClientError(String(e));
        setClientConnected(false);
        addLog("error", `Connection failed: ${String(e)}`);
      } finally {
        setClientLoading(false);
      }
    },
    [addLog, pro, clientTlsEnabled, clientCa, clientCert, clientKey, keepAlive, lwtTopic, lwtPayload, lwtQos, lwtRetain],
  );

  const handleClientDisconnect = useCallback(async () => {
    try {
      await invoke("client_disconnect");
    } catch { /* ignore */ }
    setClientConnected(false);
    setClientEndpoint(null);
    setSubscriptions([]);
    addLog("info", "Disconnected");
  }, [addLog]);

  const handleSubscribe = useCallback(async () => {
    if (!subscribeTopic || !clientConnected) return;
    if (subscriptions.some((s) => s.topic === subscribeTopic)) return;
    try {
      // Backend now awaits the broker's SUBACK and returns the granted QoS
      // (the broker may downgrade it).
      const reqQos = subscribeQos === "atleastonce" ? 1 : subscribeQos === "exactlyonce" ? 2 : 0;
      const granted = await invoke<number>("client_subscribe", { topic: subscribeTopic, qos: subscribeQos });
      const qosNum = String(granted);
      const note = granted < reqQos ? " (downgraded by broker)" : "";
      setSubscriptions((prev) => [...prev, { topic: subscribeTopic, qos: qosNum }]);
      addLog("tx", `Subscribed: ${subscribeTopic} (QoS ${qosNum}${note})`);
      setSubscribeTopic("");
    } catch (e) {
      addLog("error", `Subscribe failed: ${String(e)}`);
    }
  }, [subscribeTopic, clientConnected, subscriptions, subscribeQos, addLog]);

  const handleUnsubscribe = useCallback(async (topic: string) => {
    try {
      await invoke("client_unsubscribe", { topic });
      setSubscriptions((prev) => prev.filter((s) => s.topic !== topic));
      addLog("tx", `Unsubscribed: ${topic}`);
    } catch (e) {
      addLog("error", `Unsubscribe failed: ${String(e)}`);
    }
  }, [addLog]);

  const handlePublish = useCallback(async () => {
    if (!publishTopic || !publishPayload) return;
    try {
      await invoke("client_publish", {
        message: { topic: publishTopic, payload: publishPayload, qos: publishQos, retained: publishRetained },
      });
      addLog("tx", `Publish to ${publishTopic}: ${publishPayload}`);
    } catch (e) {
      addLog("error", `Publish failed: ${String(e)}`);
    }
  }, [publishTopic, publishPayload, publishQos, publishRetained, addLog]);

  // Publish to an explicit topic+value (used by the row editor in client mode).
  const handlePublishTo = useCallback(async (topic: string, payload: string) => {
    if (!topic || !clientConnected) return;
    try {
      await invoke("client_publish", {
        message: { topic, payload, qos: publishQos, retained: publishRetained },
      });
      addLog("tx", `Publish to ${topic}: ${payload}`);
    } catch (e) {
      addLog("error", `Publish failed: ${String(e)}`);
    }
  }, [publishQos, publishRetained, clientConnected, addLog]);


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
      setClientMessages([]);
      setClientTopicValues({});
      setBlinkingClientTopics(new Set());
    }
  };

  // Decode a payload string according to the selected format.
  const decodePayload = useCallback((payload: string): string => {
    const p = String(payload ?? "");
    switch (payloadFormat) {
      case "json": {
        try { return JSON.stringify(JSON.parse(p), null, 2); } catch { return p; }
      }
      case "hex": {
        const bytes = new TextEncoder().encode(p);
        return Array.from(bytes).map((b) => b.toString(16).padStart(2, "0")).join(" ");
      }
      case "base64": {
        const bytes = new TextEncoder().encode(p);
        let bin = "";
        for (const b of bytes) bin += String.fromCharCode(b);
        return btoa(bin);
      }
      default:
        return p;
    }
  }, [payloadFormat]);

  // Decode the payload cell, badge the retain flag, pass every other cell through unchanged.
  const renderCell = useCallback((key: string, row: Record<string, unknown>) => {
    if (key === "payload") {
      return <span style={payloadCellStyle}>{decodePayload(String(row.payload ?? ""))}</span>;
    }
    if (key === "retain") {
      return row.retain === "Yes" ? <Badge tone="info" mono>Yes</Badge> : "No";
    }
    return String(row[key] ?? "");
  }, [decodePayload]);

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

  const brokerTableData = brokerTopics.map((t) => ({
    topic: t.topic,
    payload: t.last_payload,
    count: t.message_count,
    time: fmtTs(t.last_update),
    retain: t.retain ? "Yes" : "No",
  }));
  const brokerRowStyle = (row: Record<string, unknown>) =>
    blinkingTopics.has(String(row.topic)) ? { background: "var(--accent)", color: "var(--accent-fg)" } : undefined;

  // Client-mode table: latest value per subscribed topic (parity with broker).
  const clientTopicEntries = Object.entries(clientTopicValues).map(([topic, v]) => ({
    topic,
    payload: v.payload,
    count: v.count,
    time: fmtTs(v.ts),
    retain: v.retain ? "Yes" : "No",
  }));
  const clientRowStyle = (row: Record<string, unknown>) =>
    blinkingClientTopics.has(String(row.topic)) ? { background: "var(--accent)", color: "var(--accent-fg)" } : undefined;

  // Topic-tree entries need real timestamps (the table renders pre-formatted time).
  const treeEntries: TreeEntry[] =
    mode === "broker"
      ? brokerTopics.map((t) => ({
          topic: t.topic, payload: t.last_payload, count: t.message_count,
          ts: t.last_update, retain: t.retain,
        }))
      : Object.entries(clientTopicValues).map(([topic, v]) => ({
          topic, payload: v.payload, count: v.count, ts: v.ts, retain: v.retain,
        }));
  const treeBlinking = mode === "broker" ? blinkingTopics : blinkingClientTopics;

  const historyRows = (mode === "broker" ? brokerMessages : clientMessages).map((m) => ({
    topic: String(m.topic ?? ""),
    payload: String(m.payload ?? ""),
    qos: (m.qos as number) ?? 0,
    time: fmtTs((m._ts as number) ?? m.timestamp ?? Date.now()),
    retain: m.retain ? "Yes" : "No",
  }));

  const dataCount = mode === "broker" ? brokerTopics.length : clientTopicEntries.length;
  const historyCount = mode === "broker" ? brokerMessages.length : clientMessages.length;
  const showClear =
    (dataView === "topics" && dataCount > 0) || (dataView === "history" && historyCount > 0);

  const handleExportHistory = async () => {
    if (!requirePro("Message history export")) return;
    const csv = messagesToCsv(
      (mode === "broker" ? brokerMessages : clientMessages).map((m) => ({
        timestamp: (m._ts as number) ?? m.timestamp ?? Date.now(),
        topic: String(m.topic ?? ""),
        payload: String(m.payload ?? ""),
        qos: mode === "broker" ? null : ((m.qos as number) ?? 0),
        retain: !!m.retain,
      })),
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
        onChange={setDataView}
        items={[
          { value: "topics", label: "Topics", count: dataCount },
          { value: "tree", label: "Tree", count: dataCount },
          { value: "history", label: "History", count: historyCount },
        ]}
        trailing={
          <>
            <select
              className="vt-select vt-select--sm"
              style={{ width: 96 }}
              value={payloadFormat}
              onChange={(e) => {
                const v = e.target.value as "raw" | "json" | "hex" | "base64";
                if ((v === "hex" || v === "base64") && !requirePro("Hex and Base64 payload decoders")) return;
                setPayloadFormat(v);
              }}
              aria-label="Payload format"
              title="Payload format"
            >
              <option value="raw">Raw</option>
              <option value="json">JSON</option>
              <option value="hex">{pro ? "Hex" : "Hex (Pro)"}</option>
              <option value="base64">{pro ? "Base64" : "Base64 (Pro)"}</option>
            </select>
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
            : `Message stream, ${historyCount} message${historyCount !== 1 ? "s" : ""}`}
      </span>

      {dataView === "tree" ? (
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
            formatPayload={decodePayload}
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
          <div style={{ display: "flex", gap: 4, marginBottom: 8 }}>
            <select
              className="vt-select vt-select--sm"
              style={{ flex: 1, minWidth: 0 }}
              value={selectedProfile}
              onChange={(e) => handleLoadProfile(e.target.value)}
              disabled={clientConnected}
              title="Load a saved connection profile"
              aria-label="Saved profiles"
            >
              <option value="">Saved profiles…</option>
              {profiles.map((p) => (
                <option key={p.name} value={p.name}>{p.name}</option>
              ))}
            </select>
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
                    <select className="vt-select vt-select--sm" value={lwtQos} onChange={(e) => setLwtQos(e.target.value)}>
                      <option value="atmostonce">QoS 0</option>
                      <option value="atleastonce">QoS 1</option>
                      <option value="exactlyonce">QoS 2</option>
                    </select>
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
            <select
              className="vt-select vt-select--sm"
              style={{ width: 84, flexShrink: 0 }}
              value={subscribeQos}
              onChange={(e) => setSubscribeQos(e.target.value)}
              aria-label="Subscribe QoS"
            >
              <option value="atmostonce">QoS 0</option>
              <option value="atleastonce">QoS 1</option>
              <option value="exactlyonce">QoS 2</option>
            </select>
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
                <select className="vt-select vt-select--sm" value={publishQos} onChange={(e) => setPublishQos(e.target.value)}>
                  <option value="atmostonce">QoS 0</option>
                  <option value="atleastonce">QoS 1</option>
                  <option value="exactlyonce">QoS 2</option>
                </select>
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
            broker to test against.
          </p>
          <p style={{ margin: "0 0 12px", color: "var(--text-muted)" }}>
            Part of the <strong style={{ color: "var(--text)" }}>Voltrus</strong> suite by JoyoDigitama
          </p>
          <ProAboutRow />
        </div>
      </Dialog>
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
