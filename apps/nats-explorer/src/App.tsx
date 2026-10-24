import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { SERVER_MESSAGE_CAP, serverMessagesFromRecent } from "./serverMessages";
import { jsConsumerNext, jsCreateStream } from "./nats-client";
import {
  ArrowDown,
  Eraser,
  Inbox,
  Play,
  Plug,
  Plus,
  RefreshCw,
  Send,
  Server,
  Square,
  Trash2,
  Unplug,
  X,
} from "lucide-react";
import {
  Badge,
  Button,
  Callout,
  DataTable,
  Dialog,
  EmptyState,
  Field,
  IconButton,
  Kbd,
  LogPanel,
  Section,
  Segmented,
  SheetSelect,
  StatusBar,
  StatusDot,
  Tabs,
  Toolbar,
  ProBadge,
  ProAboutRow,
  saveTextFile,
  useProStatus,
  type Column,
  type LogEntry,
} from "@mactools/ui";
import { KvPanel } from "./panels/KvPanel";
import { ObjPanel } from "./panels/ObjPanel";
import { SchemaPanel } from "./panels/SchemaPanel";
import { MonitorPanel } from "./panels/MonitorPanel";
import appIcon from "../src-tauri/icons/128x128.png";
import {
  deleteProfile,
  FREE_PROFILE_LIMIT,
  getSessionSecrets,
  loadProfiles,
  profileNeedsSecret,
  setSessionSecrets,
  upsertProfile,
  type ConnProfile,
} from "./profiles";

// ---------------------------------------------------------------------------
// Types (mirror mt-nats serde types)
// ---------------------------------------------------------------------------

interface NatsMessage {
  subject: string;
  reply?: string | null;
  headers: [string, string][];
  payload: string;
  timestamp_ms: number;
  queue?: string | null;
}

interface SubjectInfo {
  subject: string;
  message_count: number;
  last_update: number;
}

interface ServerRecent {
  subject: string;
  payload: string;
  timestamp: number;
  headers: [string, string][];
}

interface NatsStreamInfo {
  name: string;
  subjects: string[];
  description?: string | null;
  messages: number;
  bytes: number;
  max_messages: number;
  max_age_ns: number;
  storage: string;
  consumers: number;
  created?: string | null;
}

interface NatsConsumerInfo {
  stream: string;
  name: string;
  durable: boolean;
  deliver_kind: string;
  ack_policy: string;
  pending: number;
  num_ack_pending: number;
  num_redelivered: number;
}

interface NatsStoredMessage {
  sequence: number;
  subject: string;
  payload: string;
  timestamp_ms: number;
}

type SectionTab = "messages" | "js" | "kv" | "obj" | "monitor" | "schema" | "log";

// ---------------------------------------------------------------------------

function fmtTs(ts: number) {
  return new Date(ts).toLocaleTimeString("en-US", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
  });
}

// Drag handle between resizable regions.
function DragHandle({
  side,
  active,
  onStart,
}: {
  side: "left" | "right" | "table";
  active: boolean;
  onStart: (side: "left" | "right" | "table", e: React.MouseEvent) => void;
}) {
  const vertical = side !== "table";
  return (
    <div
      onMouseDown={(e) => onStart(side, e)}
      style={{
        [vertical ? "width" : "height"]: 4,
        cursor: vertical ? "col-resize" : "row-resize",
        background: active ? "var(--accent)" : "transparent",
        flexShrink: 0,
        zIndex: 10,
        transition: "background 0.15s",
      }}
      onMouseEnter={(e) => {
        if (!active) (e.target as HTMLElement).style.background = "var(--border)";
      }}
      onMouseLeave={(e) => {
        if (!active) (e.target as HTMLElement).style.background = "transparent";
      }}
    />
  );
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

const AUTH_OPTIONS = [
  { value: "none", label: "None" },
  { value: "userpass", label: "Username / Password" },
  { value: "token", label: "Token" },
  { value: "creds_file", label: "Credentials file (.creds)" },
  { value: "creds", label: "Credentials (inline)" },
  { value: "nkey", label: "NKey seed" },
  { value: "jwt", label: "User JWT + NKey" },
] as const;

export default function App() {
  const [mode, setMode] = useState<"server" | "client">("server");

  const { requirePro, status: proStatus } = useProStatus();

  // Client state
  const [clientConnected, setClientConnected] = useState(false);
  const [clientHost, setClientHost] = useState("127.0.0.1");
  const [clientPort, setClientPort] = useState(4222);
  const [clientName, setClientName] = useState("nats-explorer");
  const [clientUser, setClientUser] = useState("");
  const [clientPass, setClientPass] = useState("");
  const [clientToken, setClientToken] = useState("");
  const [clientTls, setClientTls] = useState(false);
  const [clientCa, setClientCa] = useState("");
  const [clientCert, setClientCert] = useState("");
  const [clientKey, setClientKey] = useState("");
  const [clientConnecting, setClientConnecting] = useState(false);
  const [clientAuthMode, setClientAuthMode] = useState<"none" | "userpass" | "token" | "creds_file" | "creds" | "nkey" | "jwt">("none");
  const [clientCredsPath, setClientCredsPath] = useState("");
  const [clientCredsBody, setClientCredsBody] = useState("");
  const [clientNkeySeed, setClientNkeySeed] = useState("");
  const [clientJwt, setClientJwt] = useState("");
  const [clientJwtSeed, setClientJwtSeed] = useState("");
  const [profiles, setProfiles] = useState<ConnProfile[]>(loadProfiles);
  const [selectedProfile, setSelectedProfile] = useState("");
  const [saveAsOpen, setSaveAsOpen] = useState(false);
  const [saveAsName, setSaveAsName] = useState("");
  const [profileNote, setProfileNote] = useState<string | null>(null);
  const [clientMessages, setClientMessages] = useState<(NatsMessage & { _ts: number })[]>([]);
  const [clientError, setClientError] = useState<string | null>(null);
  const [subTopic, setSubTopic] = useState("demo.>");
  const [subQueue, setSubQueue] = useState("");
  const [subs, setSubs] = useState<{ sid: string; subject: string; queue?: string }[]>([]);
  const [pubSubject, setPubSubject] = useState("demo.subject");
  const [pubPayload, setPubPayload] = useState("");
  const [pubHeaders, setPubHeaders] = useState<[string, string][]>([]);
  const [hdrKey, setHdrKey] = useState("");
  const [hdrVal, setHdrVal] = useState("");

  // Server state
  const [serverRunning, setServerRunning] = useState(false);
  const [serverPort, setServerPort] = useState(4222);
  const [serverMaxConn, setServerMaxConn] = useState(100);
  const [serverAuthEnabled, setServerAuthEnabled] = useState(false);
  const [serverUsers, setServerUsers] = useState<{ username: string; password: string }[]>([
    { username: "", password: "" },
  ]);
  const [serverMessages, setServerMessages] = useState<(ServerRecent & { _ts: number })[]>([]);
  const serverClearedAt = useRef(0);
  const [serverSubjects, setServerSubjects] = useState<SubjectInfo[]>([]);

  // JetStream state
  const [jsTab, setJsTab] = useState(false);
  const [streams, setStreams] = useState<NatsStreamInfo[]>([]);
  const [jsStream, setJsStream] = useState("");
  const [jsSubjects, setJsSubjects] = useState("events.>");
  const [jsMaxMsgs, setJsMaxMsgs] = useState(10000);
  const [jsMaxAgeDays, setJsMaxAgeDays] = useState(0);
  const [jsError, setJsError] = useState<string | null>(null);
  const [jsBusy, setJsBusy] = useState(false);
  const [streamMsgs, setStreamMsgs] = useState<NatsStoredMessage[]>([]);
  const [consumers, setConsumers] = useState<NatsConsumerInfo[]>([]);
  const [consumerName, setConsumerName] = useState("consumer");
  const [selectedStream, setSelectedStream] = useState<string | null>(null);

  // Shared
  const [log, setLog] = useState<LogEntry[]>([]);
  const [autoScroll, setAutoScroll] = useState(true);
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [mobileTab, setMobileTab] = useState<"connect" | "data" | "subs" | "js" | "kv" | "obj" | "monitor" | "schema" | "log">("connect");
  const [featureTab, setFeatureTab] = useState<"kv" | "obj" | "monitor" | "schema" | null>(null);
  const [showInfo, setShowInfo] = useState(false);
  const [infoText, setInfoText] = useState("");
  const [exportMsg, setExportMsg] = useState("");

  const [leftWidth, setLeftWidth] = useState(280);
  const [rightWidth, setRightWidth] = useState(280);
  const [tableHeight, setTableHeight] = useState(300);
  const [dragSide, setDragSide] = useState<"left" | "right" | "table" | null>(null);
  const dragStartRef = useRef({ startX: 0, startY: 0, startWidth: 0, startHeight: 0 });

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev.slice(-499), { timestamp: Date.now(), direction, message }]);
  }, []);

  // Window sizing + theme (match mqtt-explorer).
  useEffect(() => {
    const setSize = async () => {
      try {
        const win = getCurrentWindow();
        await win.setSize(new LogicalSize(1280, 820));
        await win.center();
      } catch { /* browser preview */ }
    };
    setSize();
    const savedTheme = localStorage.getItem("mactools-theme");
    if (savedTheme) document.documentElement.setAttribute("data-theme", savedTheme);
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // -------------------------------------------------------------------------
  // Drag resize
  // -------------------------------------------------------------------------
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
        setLeftWidth(Math.max(200, Math.min(420, dragStartRef.current.startWidth + (e.clientX - dragStartRef.current.startX))));
      } else if (dragSide === "right") {
        setRightWidth(Math.max(200, Math.min(420, dragStartRef.current.startWidth - (e.clientX - dragStartRef.current.startX))));
      } else if (dragSide === "table") {
        setTableHeight(Math.max(80, dragStartRef.current.startHeight + (e.clientY - dragStartRef.current.startY)));
      }
    };
    const onUp = () => setDragSide(null);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
  }, [dragSide]);

  // -------------------------------------------------------------------------
  // Event listeners
  // -------------------------------------------------------------------------
  useEffect(() => {
    let unServer: UnlistenFn | null = null;
    let unClient: UnlistenFn | null = null;
    let unDisconnect: UnlistenFn | null = null;

    listen<ServerRecent>("server-message", (event) => {
      setServerMessages((prev) => [...prev.slice(-199), { ...event.payload, _ts: Date.now() }]);
    }).then((fn) => { unServer = fn; });

    listen<NatsMessage>("client-message", (event) => {
      setClientMessages((prev) => [...prev.slice(-199), { ...event.payload, _ts: Date.now() }]);
    }).then((fn) => { unClient = fn; });

    listen<{ reason: string; detail?: string }>("client-disconnected", (event) => {
      setClientConnected(false);
      setClientError(`Connection lost: ${event.payload.detail ?? event.payload.reason}`);
      addLog("error", `Connection lost: ${event.payload.detail ?? event.payload.reason}`);
    }).then((fn) => { unDisconnect = fn; });

    return () => {
      unServer?.();
      unClient?.();
      unDisconnect?.();
    };
  }, [addLog]);

  // Server status + subjects poll.
  useEffect(() => {
    if (!serverRunning) return;
    const poll = async () => {
      try {
        const status = await invoke<string>("server_get_status");
        if (status !== "running") {
          setServerRunning(false);
          setServerSubjects([]);
          return;
        }
        const subs = await invoke<SubjectInfo[]>("server_get_subjects");
        setServerSubjects(subs);
        const recent = await invoke<ServerRecent[]>("server_get_recent", { limit: SERVER_MESSAGE_CAP });
        setServerMessages(serverMessagesFromRecent(recent, serverClearedAt.current));
      } catch { /* server stopped */ }
    };
    poll();
    const interval = setInterval(poll, 1000);
    return () => clearInterval(interval);
  }, [serverRunning]);

  // Client recent poll fallback.
  useEffect(() => {
    if (!clientConnected) return;
    const interval = setInterval(async () => {
      try {
        const recent = await invoke<NatsMessage[]>("client_get_recent");
        if (recent.length) {
          setClientMessages((prev) => {
            const seen = new Set(prev.map((m) => `${m.subject}|${m.payload}|${m.timestamp_ms}`));
            const fresh = recent.filter((r) => !seen.has(`${r.subject}|${r.payload}|${r.timestamp_ms}`));
            return [...fresh.map((r) => ({ ...r, _ts: Date.now() })), ...prev].slice(0, 200);
          });
        }
      } catch { }
    }, 1000);
    return () => clearInterval(interval);
  }, [clientConnected]);

  // -------------------------------------------------------------------------
  // Client mode handlers
  // -------------------------------------------------------------------------
  // Saved connection profiles. The free tier keeps one; secrets are never persisted (see profiles.ts).
  const openSaveProfile = () => {
    if (profiles.length >= FREE_PROFILE_LIMIT && !requirePro("More than one saved connection profile")) return;
    setSaveAsName(selectedProfile || clientHost);
    setSaveAsOpen(true);
  };

  const handleSaveProfile = () => {
    const name = saveAsName.trim();
    if (!name) return;
    const isNew = !profiles.some((x) => x.name === name);
    if (isNew && profiles.length >= FREE_PROFILE_LIMIT && !requirePro("More than one saved connection profile")) return;
    const p: ConnProfile = {
      name,
      host: clientHost,
      port: clientPort,
      clientName,
      tls: clientTls,
      authMode: clientAuthMode,
      username: clientAuthMode === "userpass" && clientUser ? clientUser : undefined,
      credsPath: clientAuthMode === "creds_file" && clientCredsPath ? clientCredsPath : undefined,
    };
    setSessionSecrets(name, {
      password: clientPass,
      token: clientToken,
      credsBody: clientCredsBody,
      nkeySeed: clientNkeySeed,
      jwt: clientJwt,
      jwtSeed: clientJwtSeed,
    });
    setProfiles(upsertProfile(p));
    setSelectedProfile(name);
    setProfileNote(null);
    setSaveAsOpen(false);
    addLog("info", `Saved profile "${name}"`);
  };

  const handleLoadProfile = (name: string) => {
    setSelectedProfile(name);
    const p = profiles.find((x) => x.name === name);
    if (!p) {
      setProfileNote(null);
      return;
    }
    setClientHost(p.host);
    setClientPort(p.port);
    setClientName(p.clientName);
    setClientTls(p.tls);
    setClientAuthMode(p.authMode);
    setClientUser(p.username ?? "");
    setClientCredsPath(p.credsPath ?? "");
    const s = getSessionSecrets(p.name);
    setClientPass(s?.password ?? "");
    setClientToken(s?.token ?? "");
    setClientCredsBody(s?.credsBody ?? "");
    setClientNkeySeed(s?.nkeySeed ?? "");
    setClientJwt(s?.jwt ?? "");
    setClientJwtSeed(s?.jwtSeed ?? "");
    setProfileNote(
      profileNeedsSecret(p) && !s
        ? "Secrets are not saved to disk. Enter the password, token, seed or credentials again to connect."
        : null,
    );
    setClientError(null);
    addLog("info", `Loaded profile "${p.name}"`);
  };

  const handleDeleteProfile = () => {
    if (!selectedProfile) return;
    const name = selectedProfile;
    setProfiles(deleteProfile(name));
    setSelectedProfile("");
    setProfileNote(null);
    addLog("info", `Deleted profile "${name}"`);
  };

  const handleClientConnect = async () => {
    setClientConnecting(true);
    setClientError(null);
    try {
      const auth =
        clientAuthMode === "userpass" ? { type: "user_pass", username: clientUser, password: clientPass }
        : clientAuthMode === "token" ? { type: "token", token: clientToken }
        : clientAuthMode === "creds_file" ? { type: "credentials_file", path: clientCredsPath }
        : clientAuthMode === "creds" ? { type: "credentials", creds: clientCredsBody }
        : clientAuthMode === "nkey" ? { type: "nkey", seed: clientNkeySeed }
        : clientAuthMode === "jwt" ? { type: "jwt", user_jwt: clientJwt, nkey_seed: clientJwtSeed }
        : { type: "none" };
      await invoke("client_connect", {
        config: {
          host: clientHost,
          port: clientPort,
          name: clientName,
          auth,
          username: null,
          password: null,
          token: null,
          tls: {
            enabled: clientTls,
            ca_pem: clientCa || null,
            client_cert_pem: clientCert || null,
            client_key_pem: clientKey || null,
          },
          connect_timeout_ms: 5000,
        },
      });
      setClientConnected(true);
      setClientMessages([]);
      addLog("info", `Connected to ${clientHost}:${clientPort}`);
    } catch (e) {
      const msg = String(e);
      setClientError(msg);
      addLog("error", `Connect failed: ${msg}`);
    }
    setClientConnecting(false);
  };

  const handleClientDisconnect = async () => {
    try {
      await invoke("client_disconnect");
      setClientConnected(false);
      setSubs([]);
      setClientMessages([]);
      addLog("info", "Disconnected");
    } catch (e) {
      addLog("error", `Disconnect failed: ${e}`);
    }
  };

  const handleSubscribe = async () => {
    if (!subTopic.trim()) return;
    try {
      const sid = await invoke<string>("client_subscribe", {
        subject: subTopic.trim(),
        queue: subQueue.trim() || null,
      });
      setSubs((prev) => [...prev, { sid, subject: subTopic.trim(), queue: subQueue.trim() || undefined }]);
      addLog("tx", `SUB ${subTopic.trim()}${subQueue.trim() ? ` (queue ${subQueue.trim()})` : ""}`);
    } catch (e) {
      addLog("error", `Subscribe failed: ${e}`);
    }
  };

  const handleUnsubscribe = async (sid: string, subject: string) => {
    try {
      await invoke("client_unsubscribe", { sid });
      setSubs((prev) => prev.filter((s) => s.sid !== sid));
      addLog("tx", `UNSUB ${subject}`);
    } catch (e) {
      addLog("error", `Unsubscribe failed: ${e}`);
    }
  };

  const handlePublish = async () => {
    if (!pubSubject.trim()) return;
    try {
      await invoke("client_publish", {
        message: {
          subject: pubSubject.trim(),
          payload: pubPayload,
          headers: pubHeaders,
        },
      });
      addLog("tx", `PUB ${pubSubject.trim()} (${pubPayload.length} bytes)`);
    } catch (e) {
      addLog("error", `Publish failed: ${e}`);
    }
  };

  // -------------------------------------------------------------------------
  // Server mode handlers
  // -------------------------------------------------------------------------
  const handleServerStart = async () => {
    try {
      const users = serverAuthEnabled ? serverUsers.filter((u) => u.username.trim()) : [];
      await invoke("server_start", {
        config: {
          enabled: true,
          port: serverPort,
          max_connections: serverMaxConn,
          auth: {
            mode: users.length ? "password" : "none",
            users,
          },
        },
      });
      setServerRunning(true);
      setServerMessages([]);
      addLog("info", `Server started on :${serverPort}`);
    } catch (e) {
      addLog("error", `Server start failed: ${e}`);
    }
  };

  const handleServerStop = async () => {
    try {
      await invoke("server_stop");
      setServerRunning(false);
      setServerSubjects([]);
      setServerMessages([]);
      addLog("info", "Server stopped");
    } catch (e) {
      addLog("error", `Server stop failed: ${e}`);
    }
  };

  const handleServerPublish = async () => {
    if (!pubSubject.trim()) return;
    try {
      await invoke("server_publish", {
        subject: pubSubject.trim(),
        payload: pubPayload,
        headers: pubHeaders,
      });
      addLog("tx", `PUB ${pubSubject.trim()} (${pubPayload.length} bytes)`);
    } catch (e) {
      addLog("error", `Publish failed: ${e}`);
    }
  };

  // -------------------------------------------------------------------------
  // JetStream handlers
  // -------------------------------------------------------------------------
  const refreshStreams = useCallback(async () => {
    try {
      const names = await invoke<string[]>("js_list_streams");
      const infos: NatsStreamInfo[] = [];
      for (const n of names) {
        const info = await invoke<NatsStreamInfo | null>("js_stream_info", { name: n });
        if (info) infos.push(info);
      }
      setStreams(infos);
      setJsError(null);
    } catch (e) {
      setJsError(String(e));
    }
  }, []);

  const handleCreateStream = async () => {
    if (!requirePro("Creating streams")) return;
    if (!jsStream.trim()) return;
    setJsBusy(true);
    setJsError(null);
    try {
      const subjects = jsSubjects.split(",").map((s) => s.trim()).filter(Boolean);
      await jsCreateStream(
        jsStream.trim(),
        subjects,
        jsMaxMsgs,
        jsMaxAgeDays > 0 ? jsMaxAgeDays * 86400 * 1e9 : 0,
      );
      await refreshStreams();
      addLog("info", `Stream created: ${jsStream.trim()}`);
      setJsStream("");
    } catch (e) {
      setJsError(String(e));
      addLog("error", `Create stream failed: ${e}`);
    }
    setJsBusy(false);
  };

  const handleSelectStream = async (name: string) => {
    if (!requirePro("Fetching stream messages")) return;
    setSelectedStream(name);
    try {
      const msgs = await invoke<NatsStoredMessage[]>("js_stream_messages", { name, limit: 50 });
      setStreamMsgs(msgs);
      const stream = streams.find((s) => s.name === name);
      if (stream) {
        const cons: NatsConsumerInfo[] = [];
        // We only track consumers we created in this session; use the UI's
        // known list pattern from create/delete.
        setConsumers(cons);
      }
    } catch (e) {
      setJsError(String(e));
    }
  };

  const handleDeleteStream = async (name: string) => {
    if (!requirePro("Deleting streams")) return;
    try {
      await invoke("js_delete_stream", { name });
      if (selectedStream === name) { setSelectedStream(null); setStreamMsgs([]); setConsumers([]); }
      await refreshStreams();
      addLog("info", `Stream deleted: ${name}`);
    } catch (e) {
      setJsError(String(e));
    }
  };

  const handlePurgeStream = async (name: string) => {
    if (!requirePro("Purging streams")) return;
    try {
      await invoke("js_purge_stream", { name });
      await refreshStreams();
      if (selectedStream === name) setStreamMsgs([]);
      addLog("info", `Stream purged: ${name}`);
    } catch (e) {
      setJsError(String(e));
    }
  };

  const handleCreateConsumer = async () => {
    if (!selectedStream || !consumerName.trim()) return;
    if (!requirePro("Creating consumers")) return;
    try {
      await invoke("js_create_consumer", { stream: selectedStream, durable: consumerName.trim() });
      const info = await invoke<NatsConsumerInfo>("js_consumer_info", {
        stream: selectedStream,
        durable: consumerName.trim(),
      });
      setConsumers((prev) => [...prev.filter((c) => c.name !== info.name), info]);
      addLog("info", `Consumer created: ${selectedStream}/${consumerName.trim()}`);
    } catch (e) {
      setJsError(String(e));
    }
  };

  const handleConsumerNext = async (stream: string, durable: string) => {
    if (!requirePro("Fetching stream messages")) return;
    try {
      const msgs = await jsConsumerNext<NatsStoredMessage>(stream, durable, 10, 3000);
      setStreamMsgs((prev) => [...msgs, ...prev].slice(0, 50));
      addLog("rx", `Consumer ${durable}: fetched ${msgs.length} messages`);
    } catch (e) {
      setJsError(String(e));
      addLog("error", `Consumer fetch failed: ${e}`);
    }
  };

  const handleDeleteConsumer = async (stream: string, durable: string) => {
    if (!requirePro("Deleting consumers")) return;
    try {
      await invoke("js_delete_consumer", { stream, durable });
      setConsumers((prev) => prev.filter((c) => !(c.stream === stream && c.name === durable)));
      addLog("info", `Consumer deleted: ${stream}/${durable}`);
    } catch (e) {
      setJsError(String(e));
    }
  };

  const handleRequest = async () => {
    const subject = pubSubject.trim();
    if (!subject) return;
    addLog("tx", `REQ ${subject} (${pubPayload.length} bytes)`);
    try {
      const reply = await invoke<NatsMessage>("client_request", {
        subject,
        payload: pubPayload,
        timeoutMs: 5000,
      });
      setClientMessages((prev) => [...prev.slice(-199), { ...reply, subject: reply.subject || subject, _ts: Date.now() }]);
      addLog("rx", `REPLY ${reply.subject} (${reply.payload.length} bytes)`);
    } catch (e) {
      addLog("error", `Request failed: ${e}`);
    }
  };

  // -------------------------------------------------------------------------
  // Info dialog
  // -------------------------------------------------------------------------
  const openInfo = (text: string) => { setInfoText(text); setShowInfo(true); };

  const csvCell = (v: string) => `"${v.replace(/"/g, '""')}"`;

  const exportCsv = async () => {
    if (!requirePro("CSV export")) return;
    const all = mode === "client" ? clientMessages : serverMessages;
    const header = "Timestamp,Subject,Payload,Headers";
    const rows = all.map((m) => {
      const h = "headers" in m
        ? m.headers.map(([k, v]) => `${k}=${v}`).join(";")
        : "";
      const ts = new Date("timestamp_ms" in m ? m.timestamp_ms : m.timestamp).toISOString();
      return [ts, csvCell(m.subject), csvCell(m.payload), csvCell(h)].join(",");
    });
    const csv = [header, ...rows].join("\n");
    const name = `nats-messages-${new Date().toISOString().slice(0, 19).replace(/:/g, "-")}.csv`;
    try {
      const path = await saveTextFile(name, csv);
      if (path === null) return;
      setExportMsg("CSV exported");
      setTimeout(() => setExportMsg(""), 2000);
    } catch (e) {
      addLog("error", `Export failed: ${e}`);
    }
  };

  // -------------------------------------------------------------------------
  // Render
  // -------------------------------------------------------------------------
  const messageColumns: Column[] = [
    { key: "timestamp_ms", label: "Time", width: "110px" },
    { key: "subject", label: "Subject", width: "240px" },
    { key: "payload", label: "Payload" },
    { key: "headers", label: "Headers", width: "200px" },
  ];

  const messageRows = (mode === "client" ? clientMessages : serverMessages).map((m) => {
    const hdrs = "headers" in m
      ? m.headers.map(([k, v]) => `${k}: ${v}`).join(", ")
      : "";
    return {
      timestamp_ms: fmtTs("timestamp_ms" in m ? m.timestamp_ms : m.timestamp),
      subject: m.subject,
      payload: m.payload.length > 200 ? m.payload.slice(0, 200) + "…" : m.payload,
      headers: hdrs,
    };
  });

  const streamColumns: Column[] = [
    { key: "name", label: "Stream", width: "140px" },
    { key: "subjects", label: "Subjects" },
    { key: "messages", label: "Msgs", width: "80px", align: "right" },
    { key: "bytes", label: "Bytes", width: "90px", align: "right" },
    { key: "consumers", label: "Consumers", width: "90px", align: "right" },
    { key: "max_messages", label: "Max Msgs", width: "90px", align: "right" },
    { key: "actions", label: "Actions", width: "200px" },
  ];

  const streamRows = streams.map((s) => ({
    name: s.name,
    subjects: s.subjects.join(", "),
    messages: s.messages.toLocaleString(),
    bytes: (s.bytes / 1024).toFixed(1) + " KB",
    consumers: s.consumers.toLocaleString(),
    max_messages: s.max_messages > 0 ? s.max_messages.toLocaleString() : "∞",
    actions: "",
    _stream: s,
  }));

  // Section switch. Replicates the original strip's state updates exactly.
  const handleSectionChange = (t: SectionTab) => {
    const proFeatures: Partial<Record<SectionTab, string>> = {
      kv: "KV store browser",
      obj: "Object store browser",
      monitor: "Server monitor",
      schema: "Protobuf schema decoding",
    };
    const proName = proFeatures[t];
    if (proName && !requirePro(proName)) return;
    const isFeature = t === "kv" || t === "obj" || t === "monitor" || t === "schema";
    setFeatureTab(isFeature ? t : null);
    if (t === "js") { setJsTab(true); refreshStreams(); } else { setJsTab(false); }
    setMobileTab(t === "messages" ? "data" : t === "log" ? "log" : (isFeature ? t : t === "js" ? "js" : "data"));
  };

  const modeSegmented = (
    <Segmented
      label="Mode"
      block
      value={mode}
      onChange={setMode}
      options={[
        { value: "server", label: "Server" },
        { value: "client", label: "Client" },
      ]}
    />
  );

  const publishBlock = (onPublish: () => void, disabled: boolean, onRequest?: () => void) => (
    <Section title="Publish">
      <Field label="Subject">
        {(id) => (
          <input
            id={id}
            className="vt-input vt-input--mono"
            value={pubSubject}
            onChange={(e) => setPubSubject(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") onPublish(); }}
          />
        )}
      </Field>
      <Field
        label="Payload"
        hint={<>Press <Kbd>Cmd</Kbd> <Kbd>Enter</Kbd> to publish</>}
      >
        {(id) => (
          <textarea
            id={id}
            className="vt-textarea"
            rows={3}
            value={pubPayload}
            onChange={(e) => setPubPayload(e.target.value)}
            placeholder="Message body"
            onKeyDown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") onPublish(); }}
            style={{ resize: "vertical" }}
          />
        )}
      </Field>
      <div style={{ display: "flex", gap: 4 }}>
        <input
          className="vt-input vt-input--sm"
          style={{ flex: 1, minWidth: 0 }}
          placeholder="Header key"
          aria-label="Header key"
          value={hdrKey}
          onChange={(e) => setHdrKey(e.target.value)}
        />
        <input
          className="vt-input vt-input--sm"
          style={{ flex: 1, minWidth: 0 }}
          placeholder="Value"
          aria-label="Header value"
          value={hdrVal}
          onChange={(e) => setHdrVal(e.target.value)}
        />
        <IconButton
          size="sm"
          label="Add header"
          onClick={() => {
            if (hdrKey.trim()) {
              setPubHeaders((p) => [...p, [hdrKey.trim(), hdrVal]]);
              setHdrKey("");
              setHdrVal("");
            }
          }}
        >
          <Plus size={14} strokeWidth={1.75} />
        </IconButton>
      </div>
      {pubHeaders.length > 0 && (
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {pubHeaders.map(([k, v], i) => (
            <div
              key={i}
              style={{
                display: "flex", justifyContent: "space-between", alignItems: "center", gap: 6,
                fontSize: "var(--fs-xs)", color: "var(--text-dim)", background: "var(--hover-overlay)",
                padding: "2px 4px 2px 8px", borderRadius: "var(--radius-sm)",
              }}
            >
              <span className="mono" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {k}: {v}
              </span>
              <IconButton
                size="sm"
                danger
                label={`Remove header ${k}`}
                onClick={() => setPubHeaders((p) => p.filter((_, j) => j !== i))}
              >
                <X size={14} strokeWidth={1.75} />
              </IconButton>
            </div>
          ))}
        </div>
      )}
      <Button variant="primary" block icon={<Send size={14} strokeWidth={1.75} />} disabled={disabled} onClick={onPublish}>
        Publish
      </Button>
      {onRequest && (
        <Button block icon={<Send size={14} strokeWidth={1.75} />} disabled={disabled} onClick={onRequest}>
          Request (wait for reply)
        </Button>
      )}
    </Section>
  );

  const leftPane = mode === "server" ? (
    <>
      <Section title="Embedded server">
        <Field label="Port">
          {(id) => (
            <input
              id={id}
              className="vt-input"
              type="number"
              value={serverPort}
              onChange={(e) => setServerPort(Number(e.target.value))}
              disabled={serverRunning}
            />
          )}
        </Field>
        <Field label="Max connections">
          {(id) => (
            <input
              id={id}
              className="vt-input"
              type="number"
              value={serverMaxConn}
              onChange={(e) => setServerMaxConn(Number(e.target.value))}
              disabled={serverRunning}
            />
          )}
        </Field>
        <label className="vt-check">
          <input
            type="checkbox"
            checked={serverAuthEnabled}
            disabled={serverRunning}
            onChange={(e) => setServerAuthEnabled(e.target.checked)}
          />
          Require username / password
        </label>
        {serverAuthEnabled && (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {serverUsers.map((u, i) => (
              <div key={i} style={{ display: "flex", gap: 4 }}>
                <input
                  className="vt-input vt-input--sm"
                  placeholder="User"
                  aria-label={`Username ${i + 1}`}
                  value={u.username}
                  disabled={serverRunning}
                  onChange={(e) => setServerUsers((prev) => prev.map((x, j) => j === i ? { ...x, username: e.target.value } : x))}
                />
                <input
                  className="vt-input vt-input--sm"
                  placeholder="Password"
                  aria-label={`Password ${i + 1}`}
                  type="password"
                  value={u.password}
                  disabled={serverRunning}
                  onChange={(e) => setServerUsers((prev) => prev.map((x, j) => j === i ? { ...x, password: e.target.value } : x))}
                />
              </div>
            ))}
            <Button
              size="sm"
              variant="ghost"
              icon={<Plus size={14} strokeWidth={1.75} />}
              onClick={() => setServerUsers((prev) => [...prev, { username: "", password: "" }])}
              disabled={serverRunning}
            >
              Add user
            </Button>
          </div>
        )}
      </Section>
      <Section>
        {!serverRunning ? (
          <Button variant="primary" block icon={<Play size={14} strokeWidth={1.75} />} onClick={handleServerStart}>
            Start server
          </Button>
        ) : (
          <Button variant="danger" block icon={<Square size={14} strokeWidth={1.75} />} onClick={handleServerStop}>
            Stop server
          </Button>
        )}
        {serverRunning && (
          <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>
            <StatusDot tone="success" label="Server running" />
            <span>
              Running on <span className="mono">nats://127.0.0.1:{serverPort}</span>
            </span>
          </div>
        )}
      </Section>
    </>
  ) : (
    <>
      <Section title="Connect to server">
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
        {profileNote && <Callout tone="info">{profileNote}</Callout>}
        <Field label="Host">
          {(id) => (
            <input
              id={id}
              className="vt-input vt-input--mono"
              value={clientHost}
              onChange={(e) => setClientHost(e.target.value)}
              disabled={clientConnected}
            />
          )}
        </Field>
        <Field label="Port">
          {(id) => (
            <input
              id={id}
              className="vt-input"
              type="number"
              value={clientPort}
              onChange={(e) => setClientPort(Number(e.target.value))}
              disabled={clientConnected}
            />
          )}
        </Field>
        <Field label="Client name">
          {(id) => (
            <input
              id={id}
              className="vt-input"
              value={clientName}
              onChange={(e) => setClientName(e.target.value)}
              disabled={clientConnected}
            />
          )}
        </Field>
        <label className="vt-check">
          <input
            type="checkbox"
            checked={clientTls}
            disabled={clientConnected}
            onChange={(e) => setClientTls(e.target.checked)}
          />
          TLS (tls://)
        </label>
        <Field label="Auth">
          {(id) => (
            <SheetSelect
              id={id}
              sheetTitle="Auth"
              value={clientAuthMode}
              options={AUTH_OPTIONS}
              onChange={(v) => setClientAuthMode(v)}
              disabled={clientConnected}
            />
          )}
        </Field>
        {clientAuthMode === "userpass" && (
          <>
            <Field label="Username">
              {(id) => (
                <input
                  id={id}
                  className="vt-input"
                  value={clientUser}
                  onChange={(e) => setClientUser(e.target.value)}
                  disabled={clientConnected}
                />
              )}
            </Field>
            <Field label="Password">
              {(id) => (
                <input
                  id={id}
                  className="vt-input"
                  type="password"
                  value={clientPass}
                  onChange={(e) => setClientPass(e.target.value)}
                  disabled={clientConnected}
                />
              )}
            </Field>
          </>
        )}
        {clientAuthMode === "token" && (
          <Field label="Token">
            {(id) => (
              <input
                id={id}
                className="vt-input"
                type="password"
                value={clientToken}
                onChange={(e) => setClientToken(e.target.value)}
                disabled={clientConnected}
              />
            )}
          </Field>
        )}
        {clientAuthMode === "creds_file" && (
          <Field label=".creds file">
            {(id) => (
              <div style={{ display: "flex", gap: 4 }}>
                <input
                  id={id}
                  className="vt-input vt-input--mono"
                  style={{ flex: 1, minWidth: 0 }}
                  value={clientCredsPath}
                  onChange={(e) => setClientCredsPath(e.target.value)}
                  disabled={clientConnected}
                  placeholder="/path/to/user.creds"
                />
                <Button
                  size="sm"
                  onClick={async () => {
                    const f = await openDialog({ filters: [{ name: "NATS Credentials", extensions: ["creds"] }, { name: "All Files", extensions: ["*"] }] });
                    if (typeof f === "string") setClientCredsPath(f);
                  }}
                  disabled={clientConnected}
                >
                  Browse
                </Button>
              </div>
            )}
          </Field>
        )}
        {clientAuthMode === "creds" && (
          <Field label="Credentials body">
            {(id) => (
              <textarea
                id={id}
                className="vt-textarea vt-textarea--mono"
                value={clientCredsBody}
                onChange={(e) => setClientCredsBody(e.target.value)}
                disabled={clientConnected}
                rows={5}
                style={{ resize: "vertical" }}
                placeholder={"-----BEGIN NATS USER JWT-----\n…\n------END NATS USER JWT------\n\n-----BEGIN USER NKEY SEED-----\n…\n------END USER NKEY SEED------"}
              />
            )}
          </Field>
        )}
        {clientAuthMode === "nkey" && (
          <Field label="NKey seed">
            {(id) => (
              <input
                id={id}
                className="vt-input vt-input--mono"
                value={clientNkeySeed}
                onChange={(e) => setClientNkeySeed(e.target.value)}
                disabled={clientConnected}
                placeholder="SU…"
              />
            )}
          </Field>
        )}
        {clientAuthMode === "jwt" && (
          <>
            <Field label="User JWT">
              {(id) => (
                <textarea
                  id={id}
                  className="vt-textarea vt-textarea--mono"
                  value={clientJwt}
                  onChange={(e) => setClientJwt(e.target.value)}
                  disabled={clientConnected}
                  rows={4}
                  style={{ resize: "vertical" }}
                />
              )}
            </Field>
            <Field label="NKey seed (signs JWT nonce)">
              {(id) => (
                <input
                  id={id}
                  className="vt-input vt-input--mono"
                  value={clientJwtSeed}
                  onChange={(e) => setClientJwtSeed(e.target.value)}
                  disabled={clientConnected}
                  placeholder="SU…"
                />
              )}
            </Field>
          </>
        )}
      </Section>
      <Section>
        {!clientConnected ? (
          <Button variant="primary" block icon={<Plug size={14} strokeWidth={1.75} />} loading={clientConnecting} onClick={handleClientConnect}>
            Connect
          </Button>
        ) : (
          <Button variant="danger" block icon={<Unplug size={14} strokeWidth={1.75} />} onClick={handleClientDisconnect}>
            Disconnect
          </Button>
        )}
        {clientError && <Callout tone="danger">{clientError}</Callout>}
      </Section>
    </>
  );

  // The status poll can hand back null when the backend is unavailable; render as empty.
  const subjects = serverSubjects ?? [];

  const rightPane = mode === "server" ? (
    <>
      <Section title="Subjects" meta={`${subjects.length}`}>
        {subjects.length === 0 ? (
          <EmptyState
            compact
            title="No subjects yet"
            description="Publish a message or connect a client and active subjects appear here."
          />
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            {subjects.slice(0, 40).map((s) => (
              <div
                key={s.subject}
                style={{
                  display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8,
                  padding: "4px 8px", borderRadius: "var(--radius-sm)", background: "var(--hover-overlay)",
                }}
              >
                <span className="mono" style={{ fontSize: "var(--fs-sm)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {s.subject}
                </span>
                <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)", fontVariantNumeric: "tabular-nums" }}>
                  {s.message_count}
                </span>
              </div>
            ))}
          </div>
        )}
      </Section>
      {publishBlock(handleServerPublish, !serverRunning || !pubSubject.trim())}
    </>
  ) : (
    <>
      <Section title="Subscriptions" meta={`${subs.length}`}>
        <Field label="Subject" hint="* matches one token, > matches the tail">
          {(id) => (
            <input
              id={id}
              className="vt-input vt-input--mono"
              value={subTopic}
              onChange={(e) => setSubTopic(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleSubscribe(); }}
              placeholder="demo.>"
            />
          )}
        </Field>
        <Field label="Queue group">
          {(id) => (
            <input
              id={id}
              className="vt-input"
              value={subQueue}
              onChange={(e) => setSubQueue(e.target.value)}
              placeholder="Optional"
            />
          )}
        </Field>
        <Button variant="primary" block disabled={!clientConnected || !subTopic.trim()} onClick={handleSubscribe}>
          Subscribe
        </Button>
        {subs.length === 0 ? (
          <EmptyState
            compact
            title="No subscriptions"
            description={clientConnected ? "Subscribe to a subject to receive messages." : "Connect to a server, then subscribe to a subject."}
          />
        ) : (
          subs.map((s) => (
            <div
              key={s.sid}
              style={{
                display: "flex", justifyContent: "space-between", alignItems: "center", gap: 6,
                padding: "4px 4px 4px 8px", borderRadius: "var(--radius-sm)", background: "var(--hover-overlay)",
              }}
            >
              <span className="mono" style={{ fontSize: "var(--fs-sm)", color: "var(--text)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {s.subject}{s.queue ? ` (${s.queue})` : ""}
              </span>
              <IconButton
                size="sm"
                danger
                label={`Unsubscribe ${s.subject}`}
                onClick={() => handleUnsubscribe(s.sid, s.subject)}
              >
                <X size={14} strokeWidth={1.75} />
              </IconButton>
            </div>
          ))
        )}
      </Section>
      {publishBlock(handlePublish, !clientConnected || !pubSubject.trim(), handleRequest)}
    </>
  );

  // JetStream panel (bottom tab)
  const jsPanel = (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      {!clientConnected ? (
        <EmptyState
          icon={<Server size={18} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a JetStream-enabled server in Client mode to list streams."
        />
      ) : (
        <>
          <div className="vt-toolbar-row" style={{ flexWrap: "wrap", rowGap: 8 }}>
            <Field label="Name">
              {(id) => (
                <input
                  id={id}
                  className="vt-input vt-input--sm"
                  style={{ width: 120 }}
                  value={jsStream}
                  onChange={(e) => setJsStream(e.target.value)}
                  placeholder="events"
                />
              )}
            </Field>
            <Field label="Subjects">
              {(id) => (
                <input
                  id={id}
                  className="vt-input vt-input--sm"
                  style={{ width: 180 }}
                  value={jsSubjects}
                  onChange={(e) => setJsSubjects(e.target.value)}
                  placeholder="events.>, comma separated"
                />
              )}
            </Field>
            <Field label="Max msgs">
              {(id) => (
                <input
                  id={id}
                  className="vt-input vt-input--sm"
                  style={{ width: 80 }}
                  type="number"
                  value={jsMaxMsgs}
                  onChange={(e) => setJsMaxMsgs(Number(e.target.value))}
                />
              )}
            </Field>
            <Field label="Max age (days)">
              {(id) => (
                <input
                  id={id}
                  className="vt-input vt-input--sm"
                  style={{ width: 80 }}
                  type="number"
                  value={jsMaxAgeDays}
                  onChange={(e) => setJsMaxAgeDays(Number(e.target.value))}
                />
              )}
            </Field>
            <Button size="sm" variant="primary" loading={jsBusy} onClick={handleCreateStream} disabled={!jsStream.trim()}>
              Create stream <ProBadge />
            </Button>
            <div style={{ flex: 1 }} />
            <Button size="sm" variant="ghost" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={refreshStreams}>
              Refresh
            </Button>
          </div>
          {jsError && (
            <div style={{ padding: "6px 12px 0" }}>
              <Callout tone="danger">{jsError}</Callout>
            </div>
          )}
          <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
            <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, padding: 8 }}>
              <DataTable
                columns={streamColumns}
                data={streamRows}
                emptyMessage="No streams. Create one above to store messages."
                selectable
                renderCell={(key, row, _i) => {
                  // DataTable uses renderCell for every column; keep the default text for data columns.
                  if (key !== "actions") return String(row[key] ?? "");
                  // Rows are built in streamRows above; _stream carries the stream info.
                  const s = row["_stream"] as NatsStreamInfo | undefined;
                  if (!s) return undefined;
                  return (
                    <div style={{ display: "flex", gap: 4 }}>
                      <Button
                        size="sm"
                        variant="ghost"
                        icon={<Inbox size={14} strokeWidth={1.75} />}
                        onClick={(e) => { e.stopPropagation(); handleSelectStream(s.name); }}
                      >
                        Messages <ProBadge />
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        icon={<Eraser size={14} strokeWidth={1.75} />}
                        onClick={(e) => { e.stopPropagation(); handlePurgeStream(s.name); }}
                      >
                        Purge <ProBadge />
                      </Button>
                      <Button
                        size="sm"
                        variant="danger"
                        icon={<Trash2 size={14} strokeWidth={1.75} />}
                        onClick={(e) => { e.stopPropagation(); handleDeleteStream(s.name); }}
                      >
                        Delete <ProBadge />
                      </Button>
                    </div>
                  );
                }}
              />
            </div>
            {selectedStream && (
              <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, borderLeft: "1px solid var(--border)" }}>
                <div className="vt-toolbar-row">
                  <span className="vt-panel__title">
                    <span className="mono">{selectedStream}</span> messages
                  </span>
                  <IconButton
                    size="sm"
                    label="Close stream messages"
                    onClick={() => { setSelectedStream(null); setStreamMsgs([]); setConsumers([]); }}
                  >
                    <X size={14} strokeWidth={1.75} />
                  </IconButton>
                </div>
                <div className="vt-toolbar-row" style={{ gap: 8 }}>
                  <input
                    className="vt-input vt-input--sm"
                    style={{ width: 140 }}
                    value={consumerName}
                    onChange={(e) => setConsumerName(e.target.value)}
                    placeholder="Consumer name"
                    aria-label="Consumer name"
                  />
                  <Button size="sm" variant="primary" icon={<Plus size={14} strokeWidth={1.75} />} onClick={handleCreateConsumer}>
                    Create consumer
                  </Button>
                </div>
                <div
                  style={{
                    padding: "4px 12px", display: "flex", gap: 6, flexWrap: "wrap",
                    borderBottom: "1px solid var(--border)", maxHeight: 120, overflow: "auto", flexShrink: 0,
                  }}
                >
                  {consumers.length === 0 && (
                    <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                      No consumers yet. Create one to fetch stored messages.
                    </span>
                  )}
                  {consumers.map((c) => (
                    <div
                      key={c.name}
                      style={{
                        display: "flex", alignItems: "center", gap: 4, padding: "2px 4px 2px 8px",
                        background: "var(--hover-overlay)", borderRadius: "var(--radius-sm)", fontSize: "var(--fs-xs)",
                      }}
                    >
                      <span className="mono" style={{ color: "var(--text)" }}>{c.name}</span>
                      <span style={{ color: "var(--text-muted)", fontVariantNumeric: "tabular-nums" }}>
                        {c.pending} pending
                      </span>
                      <Button size="sm" variant="ghost" onClick={() => handleConsumerNext(selectedStream, c.name)}>
                        Fetch 10
                      </Button>
                      <IconButton
                        size="sm"
                        danger
                        label={`Delete consumer ${c.name}`}
                        onClick={() => handleDeleteConsumer(selectedStream, c.name)}
                      >
                        <X size={14} strokeWidth={1.75} />
                      </IconButton>
                    </div>
                  ))}
                </div>
                <div style={{ flex: 1, minHeight: 0, padding: 8, display: "flex", flexDirection: "column" }}>
                  <DataTable
                    columns={[
                      { key: "sequence", label: "Seq", width: "70px", align: "right" },
                      { key: "subject", label: "Subject", width: "200px" },
                      { key: "payload", label: "Payload" },
                      { key: "timestamp_ms", label: "Time", width: "110px" },
                    ]}
                    data={streamMsgs.map((m) => ({
                      sequence: m.sequence,
                      subject: m.subject,
                      payload: m.payload.length > 200 ? m.payload.slice(0, 200) + "…" : m.payload,
                      timestamp_ms: fmtTs(m.timestamp_ms),
                    }))}
                    emptyMessage="No messages fetched yet. Create a consumer and fetch a batch."
                  />
                </div>
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );

  // Active section for the tab strip.
  const view: SectionTab = featureTab ?? (jsTab ? "js" : mobileTab === "log" ? "log" : "messages");
  const msgCount = (mode === "client" ? clientMessages : serverMessages).length;

  const sectionTabs = (
    <Tabs
      label="Section"
      value={view}
      onChange={handleSectionChange}
      items={[
        { value: "messages", label: "Messages", count: msgCount },
        { value: "js", label: <>JetStream</>, count: streams.length },
        { value: "kv", label: <>KV <ProBadge /></> },
        { value: "obj", label: <>Objects <ProBadge /></> },
        { value: "monitor", label: <>Monitor <ProBadge /></> },
        { value: "schema", label: <>Schema <ProBadge /></> },
        { value: "log", label: "Log", count: log.length },
      ]}
      trailing={
        view === "messages" || view === "log" ? (
          <>
            {exportMsg && <Badge tone="accent">{exportMsg}</Badge>}
            <Button
              size="sm"
              variant="ghost"
              icon={<ArrowDown size={14} strokeWidth={1.75} />}
              aria-pressed={autoScroll}
              onClick={() => setAutoScroll((v) => !v)}
            >
              Auto-scroll
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={<Trash2 size={14} strokeWidth={1.75} />}
              onClick={() => {
                if (mode === "client") {
                  setClientMessages([]);
                } else {
                  serverClearedAt.current = Date.now();
                  setServerMessages([]);
                }
              }}
            >
              Clear
            </Button>
          </>
        ) : undefined
      }
    />
  );

  // Mobile bottom nav: plain views set the tab directly; section values take
  // the same path as the section strip (JetStream refreshes streams there).
  const handleMobileNav = (t: "connect" | "data" | "subs" | SectionTab) => {
    if (t === "connect" || t === "data" || t === "subs") {
      setMobileTab(t);
      return;
    }
    handleSectionChange(t);
  };

  const saveProfileDialog = (
    <Dialog
      open={saveAsOpen}
      onClose={() => setSaveAsOpen(false)}
      title="Save connection profile"
      width={360}
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
        Saves host, port, client name, TLS, auth type, username and the .creds file path on this Mac.
        Passwords, tokens, NKey seeds, JWTs and inline credentials are never written to disk, so you
        enter them again after restarting the app.
      </p>
    </Dialog>
  );

  const aboutDialog = (
    <Dialog
      open={showInfo}
      onClose={() => setShowInfo(false)}
      title="About NATS Explorer"
      width={460}
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
        <div style={{ whiteSpace: "pre-wrap", marginBottom: 12 }}>{infoText}</div>
        <p style={{ margin: "0 0 12px", color: "var(--text-muted)" }}>
          Part of the <strong style={{ color: "var(--text)" }}>Voltrus</strong> suite by JoyoDigitama
        </p>
        <ProAboutRow />
      </div>
    </Dialog>
  );

  if (isMobile) {
    return (
      <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)", overflow: "hidden" }}>
        <Toolbar
          title="NATS Explorer"
          icon={appIcon}
          connected={mode === "client" ? clientConnected : serverRunning}
          onDisconnect={mode === "client" ? handleClientDisconnect : handleServerStop}
          onExport={exportCsv}
          onInfo={() => openInfo(INFO_TEXT)}
        />
        <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
          <div style={{ padding: "8px 12px 0" }}>{modeSegmented}</div>
          <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
            {mobileTab === "connect" ? (
              leftPane
            ) : mobileTab === "subs" ? (
              rightPane
            ) : mobileTab === "js" ? (
              jsPanel
            ) : mobileTab === "kv" ? (
              <KvPanel connected={clientConnected} addLog={addLog} />
            ) : mobileTab === "obj" ? (
              <ObjPanel connected={clientConnected} addLog={addLog} />
            ) : mobileTab === "monitor" ? (
              <MonitorPanel connected={clientConnected} addLog={addLog} />
            ) : mobileTab === "schema" ? (
              <SchemaPanel addLog={addLog} />
            ) : (
              <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
                <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
                  <DataTable
                    columns={messageColumns}
                    data={messageRows}
                    autoScroll={autoScroll}
                    emptyMessage={mode === "client" ? EMPTY_MSG_CLIENT : EMPTY_MSG_SERVER}
                  />
                </div>
                <LogPanel entries={log} onClear={() => setLog([])} />
              </div>
            )}
          </div>
          <Tabs
            label="Mobile sections"
            value={mobileTab}
            onChange={handleMobileNav}
            items={[
              { value: "connect", label: "Connect" },
              { value: "data", label: "Data" },
              { value: "subs", label: "Subs" },
              { value: "js", label: <>JetStream</> },
              { value: "kv", label: <>KV <ProBadge /></> },
              { value: "obj", label: <>Objects <ProBadge /></> },
              { value: "monitor", label: <>Monitor <ProBadge /></> },
              { value: "schema", label: <>Schema <ProBadge /></> },
              { value: "log", label: "Log" },
            ]}
          />
        </div>
        <StatusBar
          connected={mode === "client" ? clientConnected : serverRunning}
          error={clientError}
          device={mode === "client" ? { ip: clientHost, port: clientPort } : { ip: "127.0.0.1", port: serverPort }}
        />
        {aboutDialog}
        {saveProfileDialog}
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)", overflow: "hidden" }}>
      <Toolbar
        title="NATS Explorer"
        icon={appIcon}
        connected={mode === "client" ? clientConnected : serverRunning}
        context={
          mode === "client"
            ? `nats://${clientHost}:${clientPort}`
            : serverRunning
              ? `nats://127.0.0.1:${serverPort}`
              : undefined
        }
        onDisconnect={mode === "client" ? handleClientDisconnect : handleServerStop}
        onExport={exportCsv}
        onInfo={() => openInfo(INFO_TEXT)}
      />

      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        {/* Left pane */}
        <aside className="vt-sidebar" style={{ width: leftWidth, flexShrink: 0, borderRight: "1px solid var(--border)" }}>
          <div style={{ padding: "12px 14px 0" }}>{modeSegmented}</div>
          {leftPane}
        </aside>
        <DragHandle side="left" active={dragSide === "left"} onStart={handleDragStart} />

        {/* Center */}
        <main style={{ display: "flex", flex: 1, flexDirection: "column", minWidth: 0, padding: 8, gap: 0 }}>
          {sectionTabs}
          {view === "kv" ? (
            <div style={{ flex: 1, minHeight: 0 }}>
              <KvPanel connected={clientConnected} addLog={addLog} />
            </div>
          ) : view === "obj" ? (
            <div style={{ flex: 1, minHeight: 0 }}>
              <ObjPanel connected={clientConnected} addLog={addLog} />
            </div>
          ) : view === "monitor" ? (
            <div style={{ flex: 1, minHeight: 0 }}>
              <MonitorPanel connected={clientConnected} addLog={addLog} />
            </div>
          ) : view === "schema" ? (
            <div style={{ flex: 1, minHeight: 0 }}>
              <SchemaPanel addLog={addLog} />
            </div>
          ) : view === "js" ? (
            <div style={{ flex: 1, minHeight: 0 }}>{jsPanel}</div>
          ) : (
            <>
              <div style={{ height: tableHeight, minHeight: 120, flexShrink: 0, display: "flex", flexDirection: "column" }}>
                <DataTable
                  columns={messageColumns}
                  data={messageRows}
                  autoScroll={autoScroll}
                  emptyMessage={mode === "client" ? EMPTY_MSG_CLIENT : EMPTY_MSG_SERVER}
                />
              </div>
              <DragHandle side="table" active={dragSide === "table"} onStart={handleDragStart} />
              <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
                <LogPanel entries={log} onClear={() => setLog([])} />
              </div>
            </>
          )}
        </main>

        <DragHandle side="right" active={dragSide === "right"} onStart={handleDragStart} />

        {/* Right pane */}
        <aside className="vt-sidebar" style={{ width: rightWidth, flexShrink: 0, borderLeft: "1px solid var(--border)" }}>
          {rightPane}
        </aside>
      </div>

      <StatusBar
        connected={mode === "client" ? clientConnected : serverRunning}
        error={clientError}
        device={mode === "client" ? { ip: clientHost, port: clientPort } : { ip: "127.0.0.1", port: serverPort }}
      />

        {aboutDialog}
        {saveProfileDialog}
    </div>
  );
}

const EMPTY_MSG_CLIENT = "No messages yet. Subscribe to a subject or publish one to see it here.";
const EMPTY_MSG_SERVER = "No messages yet. Start the server and publish to see traffic here.";

const INFO_TEXT = [
  "NATS Explorer inspects and publishes to NATS servers, or runs an embedded server.",
  "",
  "Server: run a local NATS server on this machine (demo/dev).",
  "Client: connect to any NATS server, subscribe with wildcards, publish with headers, send requests and read the reply.",
  "",
  "JetStream: create, delete and purge streams, create pull consumers and fetch batches. Requires a JetStream-enabled server.",
].join("\n");
