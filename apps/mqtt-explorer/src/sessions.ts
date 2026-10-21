// One client connection and everything received on it. The app keeps a list of
// these so several brokers can be watched at once.

import type { Decoded, PayloadInfo } from "./payload";

export interface Subscription {
  topic: string;
  qos: string;
}

/** Connection form and advanced settings, so switching connections restores them. */
export interface FormSnapshot {
  values: Record<string, string | number>;
  tls: boolean;
  ca: string;
  cert: string;
  key: string;
  keepAlive: number;
  lwtTopic: string;
  lwtPayload: string;
  lwtQos: string;
  lwtRetain: boolean;
}

export interface TopicValue extends PayloadInfo {
  ts: number;
  retain: boolean;
  count: number;
  /** A retained message was delivered for this topic and not cleared since. */
  retainedSeen: boolean;
}

export interface ClientMsg extends PayloadInfo {
  topic: string;
  qos: number;
  retain: boolean;
  ts: number;
}

export type SessionStatus = "idle" | "connecting" | "connected";

export interface Session {
  id: string;
  status: SessionStatus;
  error: string | null;
  endpoint: { ip: string; port: number } | null;
  /** MQTT client id used for the open connection. */
  clientId: string | null;
  /** Shown on the connection chip. */
  label: string;
  rtt: number | null;
  subscriptions: Subscription[];
  messages: ClientMsg[];
  topicValues: Record<string, TopicValue>;
  blinking: Set<string>;
  form: FormSnapshot;
}

export const HISTORY_LIMIT = 500;

export const DEFAULT_FORM: FormSnapshot = {
  values: { host: "127.0.0.1", port: 1883, clientId: "mactools-explorer", username: "", password: "" },
  tls: false,
  ca: "",
  cert: "",
  key: "",
  keepAlive: 30,
  lwtTopic: "",
  lwtPayload: "",
  lwtQos: "atmostonce",
  lwtRetain: false,
};

export function newSession(id: string): Session {
  return {
    id,
    status: "idle",
    error: null,
    endpoint: null,
    clientId: null,
    label: "New connection",
    rtt: null,
    subscriptions: [],
    messages: [],
    topicValues: {},
    blinking: new Set(),
    form: { ...DEFAULT_FORM, values: { ...DEFAULT_FORM.values } },
  };
}

/** Next free connection id (`c1`, `c2`, …). */
export function nextSessionId(sessions: Session[]): string {
  let n = 1;
  while (sessions.some((s) => s.id === `c${n}`)) n++;
  return `c${n}`;
}

export interface IncomingMessage extends PayloadInfo {
  topic: string;
  qos: number;
  retain: boolean;
  ts: number;
}

/** Record a received message: history, latest value per topic, retained tracking. */
export function applyMessage(s: Session, m: IncomingMessage): Session {
  const ex = s.topicValues[m.topic];
  const { topic, qos, retain, ts, ...info } = m;
  const msg: ClientMsg = { topic, qos, retain, ts, ...info };
  return {
    ...s,
    messages: [...s.messages.slice(-(HISTORY_LIMIT - 1)), msg],
    topicValues: {
      ...s.topicValues,
      [topic]: {
        ...info,
        ts,
        retain,
        count: (ex?.count ?? 0) + 1,
        // Only a retained delivery sets this; a live update does not say whether the stored message changed.
        retainedSeen: retain || (ex?.retainedSeen ?? false),
      },
    },
  };
}

/** Forget received data but keep the connection. */
export function clearData(s: Session): Session {
  return { ...s, messages: [], topicValues: {}, blinking: new Set() };
}

/** Topics in `s` whose retained message is still on the broker as far as we know. */
export function retainedTopics(s: Session, matches: (topic: string) => boolean): string[] {
  return Object.entries(s.topicValues)
    .filter(([topic, v]) => v.retainedSeen && matches(topic))
    .map(([topic]) => topic)
    .sort();
}

/** Mark topics as cleared on the broker. */
export function markRetainedCleared(s: Session, topics: string[]): Session {
  const tv = { ...s.topicValues };
  for (const t of topics) if (tv[t]) tv[t] = { ...tv[t], retainedSeen: false };
  return { ...s, topicValues: tv };
}

/** MQTT topic filter match (`+` one level, `#` the rest). */
export function topicMatches(filter: string, topic: string): boolean {
  if (topic.startsWith("$") && (filter.startsWith("#") || filter.startsWith("+"))) return false;
  const f = filter.split("/");
  const t = topic.split("/");
  for (let i = 0; i < f.length; i++) {
    if (f[i] === "#") return true;
    if (i >= t.length) return false;
    if (f[i] !== "+" && f[i] !== t[i]) return false;
  }
  return f.length === t.length;
}

/** Backend result of re-decoding one stored message. */
export interface Redecoded {
  id: number;
  decoded: Decoded | null;
}
