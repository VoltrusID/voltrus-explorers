// Typed wrappers over the Tauri commands in `apps/nats-explorer/src-tauri/src/lib.rs`.
import { invoke } from "@tauri-apps/api/core";

import type {
  NatsKvBucketInfo,
  NatsKvEntry,
  NatsObjectInfo,
  PayloadFormat,
} from "./nats-types";

// ----- JetStream -----
// Tauri maps each command argument from camelCase, so snake_case keys are rejected.

export const jsCreateStream = (
  name: string,
  subjects: string[],
  maxMessages: number,
  maxAgeNs: number,
) => invoke<unknown>("js_create_stream", { name, subjects, maxMessages, maxAgeNs });

export const jsConsumerNext = <T>(
  stream: string,
  durable: string,
  batch: number,
  waitMs: number,
) => invoke<T[]>("js_consumer_next", { stream, durable, batch, waitMs });

// ----- Decode + schema -----

export const decodeDetect = (payload: string) =>
  invoke<PayloadFormat>("decode_detect", { payload });

export const decodeFormat = (
  payload: string,
  format: string,
  messageFullName: string | null = null,
) => invoke<string>("decode_format", { payload, format, messageFullName });

export const schemaDecode = (subject: string, payload: string) =>
  invoke<string>("schema_decode", { subject, payload });

export const schemaAddProtoSource = (src: string) =>
  invoke<void>("schema_add_proto_source", { src });

export const schemaAddProtoFile = (path: string) =>
  invoke<void>("schema_add_proto_file", { path });

export const schemaBind = (pattern: string, messageFullName: string) =>
  invoke<void>("schema_bind", { pattern, messageFullName });

export const schemaUnbind = (pattern: string) =>
  invoke<boolean>("schema_unbind", { pattern });

export const schemaBindings = () =>
  invoke<Record<string, string>>("schema_bindings");

export const schemaMessageNames = () => invoke<string[]>("schema_message_names");

// ----- KV -----

export const kvList = () => invoke<string[]>("kv_list");
export const kvCreate = (bucket: string, history: number, maxAgeNs: number) =>
  invoke<void>("kv_create", { bucket, history, maxAgeNs });
export const kvDeleteBucket = (bucket: string) =>
  invoke<void>("kv_delete_bucket", { bucket });
export const kvStatus = (bucket: string) =>
  invoke<NatsKvBucketInfo>("kv_status", { bucket });
export const kvKeys = (bucket: string) => invoke<string[]>("kv_keys", { bucket });
export const kvGet = (bucket: string, key: string) =>
  invoke<NatsKvEntry | null>("kv_get", { bucket, key });
export const kvPut = (bucket: string, key: string, payload: string) =>
  invoke<number>("kv_put", { bucket, key, payload });
export const kvDeleteKey = (bucket: string, key: string) =>
  invoke<void>("kv_delete_key", { bucket, key });
export const kvHistory = (bucket: string, key: string) =>
  invoke<NatsKvEntry[]>("kv_history", { bucket, key });
export const kvWatchSubscribe = (bucket: string, key: string | null) =>
  invoke<string>("kv_watch_subscribe", { bucket, key });

// ----- Object Store -----

export const objList = () => invoke<string[]>("obj_list");
export const objCreate = (bucket: string) => invoke<void>("obj_create", { bucket });
export const objDeleteBucket = (bucket: string) =>
  invoke<void>("obj_delete_bucket", { bucket });
export const objListObjects = (bucket: string) =>
  invoke<NatsObjectInfo[]>("obj_list_objects", { bucket });
export const objInfo = (bucket: string, name: string) =>
  invoke<NatsObjectInfo>("obj_info", { bucket, name });
export const objGet = (bucket: string, name: string) =>
  invoke<string>("obj_get", { bucket, name });
export const objPutPayload = (bucket: string, name: string, payload: string) =>
  invoke<void>("obj_put_payload", { bucket, name, payload });
export const objUpload = (bucket: string, name: string, path: string) =>
  invoke<void>("obj_upload", { bucket, name, path });
export const objDownload = (bucket: string, name: string, path: string) =>
  invoke<void>("obj_download", { bucket, name, path });
export const objDelete = (bucket: string, name: string) =>
  invoke<void>("obj_delete", { bucket, name });

// ----- $SYS monitoring -----

export const sysSubscribe = (subject: string) =>
  invoke<string>("sys_subscribe", { subject });
export const sysRequest = (subject: string, timeoutMs: number) =>
  invoke<string>("sys_request", { subject, timeoutMs });
