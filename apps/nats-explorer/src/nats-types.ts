// TypeScript mirrors of the `mt-nats` serde types + new feature surfaces.

export interface NatsMessage {
  subject: string;
  reply?: string | null;
  headers: [string, string][];
  payload: string;
  timestamp_ms: number;
  queue?: string | null;
}

export interface NatsStoredMessage {
  sequence: number;
  subject: string;
  payload?: string;
  timestamp_ms?: number;
}

export type PayloadFormat =
  | "utf8"
  | "json"
  | "msg_pack"
  | "protobuf"
  | "hex"
  | "binary";

// ----- KV -----

export interface NatsKvBucketInfo {
  bucket: string;
  values?: number;
  bytes?: number;
  history?: number;
}

export interface NatsKvEntry {
  bucket: string;
  key: string;
  payload?: string;
  revision?: number;
  operation?: string;
  created_ms?: number;
}

// ----- Object Store -----

export interface NatsObjectInfo {
  bucket: string;
  name: string;
  size?: number;
  chunks?: number;
  digest?: string | null;
  modified_ms?: number;
  deleted?: boolean;
}

// ----- $SYS monitoring -----

export interface SysMessage {
  subject: string;
  payload: string;
  timestamp_ms: number;
}
