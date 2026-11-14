// Sparkplug B view model: edge nodes, their devices and the latest value of
// every metric, built from the backend's decoded `spBv1.0/...` messages.
// Seq/bdSeq checks are done by the backend decoder; their findings arrive in
// `checks` and are kept here per edge node.

import type { Decoded } from "./payload";

export interface SpMetric {
  name: string;
  alias?: number | string;
  datatype?: string;
  value: unknown;
  /** Metric timestamp, or the payload timestamp when the metric has none. */
  ts?: number;
}

export interface SpDevice {
  id: string;
  online: boolean;
  metrics: Record<string, SpMetric>;
  lastUpdate: number;
}

export interface SpIssue {
  ts: number;
  topic: string;
  text: string;
}

export interface SpEdgeNode {
  group: string;
  id: string;
  /** `undefined` until a NBIRTH or NDEATH is seen. */
  online?: boolean;
  bdSeq?: number;
  seq?: number;
  births: number;
  metrics: Record<string, SpMetric>;
  devices: Record<string, SpDevice>;
  lastUpdate: number;
  issues: SpIssue[];
}

export interface SpHost {
  id: string;
  online: boolean;
  ts: number;
}

export interface SparkplugState {
  /** Keyed by `group/edge`. */
  nodes: Record<string, SpEdgeNode>;
  hosts: Record<string, SpHost>;
}

export const EMPTY_SPARKPLUG: SparkplugState = { nodes: {}, hosts: {} };

const ISSUE_LIMIT = 20;
const NAMESPACE = "spBv1.0/";

interface DecodedMetric {
  name?: string;
  alias?: number | string;
  datatype?: string;
  value?: unknown;
  timestamp?: number | string;
}

interface DecodedPayload {
  topic?: { group_id?: string; message_type?: string; edge_node_id?: string; device_id?: string };
  timestamp?: number | string;
  seq?: number | string;
  bd_seq?: number | string;
  metrics?: DecodedMetric[];
  checks?: string[];
}

function num(v: unknown): number | undefined {
  if (typeof v === "number") return v;
  if (typeof v === "string" && v.trim() !== "" && !Number.isNaN(Number(v))) return Number(v);
  return undefined;
}

/** `STATE` payloads: Sparkplug 3.0 JSON (`{"online":true,...}`) or the older `ONLINE`/`OFFLINE` text. */
export function parseStatePayload(payload: string): boolean | undefined {
  const t = payload.trim();
  if (t === "ONLINE") return true;
  if (t === "OFFLINE") return false;
  try {
    const j = JSON.parse(t);
    if (j && typeof j.online === "boolean") return j.online;
  } catch {
    // not JSON
  }
  return undefined;
}

function mergeMetrics(
  into: Record<string, SpMetric>,
  metrics: DecodedMetric[],
  payloadTs: number | undefined,
  replace: boolean,
): Record<string, SpMetric> {
  const out = replace ? {} : { ...into };
  for (const m of metrics) {
    const name = m.name ?? (m.alias !== undefined ? `alias ${m.alias}` : undefined);
    if (!name) continue;
    const prev = out[name];
    out[name] = {
      name,
      alias: m.alias ?? prev?.alias,
      datatype: m.datatype ?? prev?.datatype,
      value: m.value,
      ts: num(m.timestamp) ?? payloadTs,
    };
  }
  return out;
}

/**
 * Fold one received message into the Sparkplug model. Returns `state`
 * unchanged for topics outside the namespace or payloads that did not decode.
 */
export function applySparkplug(
  state: SparkplugState,
  topic: string,
  payload: string,
  decoded: Decoded | undefined,
  ts: number,
): SparkplugState {
  if (!topic.startsWith(NAMESPACE)) return state;
  const parts = topic.split("/");
  if (parts[1] === "STATE" && parts.length === 3) {
    if (payload === "") return state;
    const online = parseStatePayload(payload);
    if (online === undefined) return state;
    return { ...state, hosts: { ...state.hosts, [parts[2]]: { id: parts[2], online, ts } } };
  }
  if (!decoded || decoded.kind !== "sparkplug" || decoded.value === undefined) return state;
  const v = decoded.value as DecodedPayload;
  const group = v.topic?.group_id;
  const edge = v.topic?.edge_node_id;
  const type = v.topic?.message_type;
  if (!group || !edge || !type) return state;
  if (type === "NCMD" || type === "DCMD") return state;

  const key = `${group}/${edge}`;
  const prev: SpEdgeNode = state.nodes[key] ?? {
    group,
    id: edge,
    births: 0,
    metrics: {},
    devices: {},
    lastUpdate: ts,
    issues: [],
  };
  const node: SpEdgeNode = { ...prev, lastUpdate: ts };
  const payloadTs = num(v.timestamp);
  const metrics = v.metrics ?? [];
  const seq = num(v.seq);
  const deviceId = v.topic?.device_id;

  switch (type) {
    case "NBIRTH":
      node.online = true;
      node.bdSeq = num(v.bd_seq);
      node.seq = seq;
      node.births = prev.births + 1;
      node.metrics = mergeMetrics({}, metrics, payloadTs, true);
      // A new session: devices must be born again.
      node.devices = Object.fromEntries(
        Object.entries(prev.devices).map(([id, d]) => [id, { ...d, online: false }]),
      );
      break;
    case "NDEATH": {
      const bd = num(v.bd_seq);
      // A death from an older session (bdSeq mismatch) does not end this one.
      if (bd !== undefined && node.bdSeq !== undefined && bd !== node.bdSeq) break;
      node.online = false;
      node.devices = Object.fromEntries(
        Object.entries(prev.devices).map(([id, d]) => [id, { ...d, online: false }]),
      );
      break;
    }
    case "NDATA":
      if (seq !== undefined) node.seq = seq;
      node.metrics = mergeMetrics(prev.metrics, metrics, payloadTs, false);
      break;
    case "DBIRTH":
    case "DDATA":
    case "DDEATH": {
      if (!deviceId) return state;
      if (seq !== undefined) node.seq = seq;
      const d: SpDevice = prev.devices[deviceId] ?? { id: deviceId, online: false, metrics: {}, lastUpdate: ts };
      node.devices = {
        ...prev.devices,
        [deviceId]: {
          ...d,
          lastUpdate: ts,
          online: type === "DDEATH" ? false : type === "DBIRTH" ? true : d.online,
          metrics:
            type === "DDEATH" ? d.metrics : mergeMetrics(d.metrics, metrics, payloadTs, type === "DBIRTH"),
        },
      };
      break;
    }
    default:
      return state;
  }

  if (v.checks && v.checks.length > 0) {
    node.issues = [...prev.issues, ...v.checks.map((text) => ({ ts, topic, text }))].slice(-ISSUE_LIMIT);
  }
  return { ...state, nodes: { ...state.nodes, [key]: node } };
}

/** Edge nodes sorted by group, then id. */
export function sortedNodes(state: SparkplugState): SpEdgeNode[] {
  return Object.values(state.nodes).sort((a, b) =>
    a.group === b.group ? a.id.localeCompare(b.id) : a.group.localeCompare(b.group),
  );
}

/** Metrics sorted by name, with folder-style names (`Line/Speed`) kept together. */
export function sortedMetrics(metrics: Record<string, SpMetric>): SpMetric[] {
  return Object.values(metrics).sort((a, b) => a.name.localeCompare(b.name));
}

export function sparkplugNodeCount(state: SparkplugState): number {
  return Object.keys(state.nodes).length;
}
