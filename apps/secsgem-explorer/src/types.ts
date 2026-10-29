// Shapes returned by the Rust commands (src-tauri/src/core.rs).

export interface AckView {
  code: number;
  name: string;
  ok: boolean;
}

export interface ControlView {
  code: number;
  name: string;
  online: boolean;
}

export interface ValueView {
  text: string;
  number: number | null;
  /** SECS-II item type: U4, A, L... */
  kind: string;
}

export interface ConnInfo {
  state: "listening" | "selected";
  peer: string;
}

export interface EstablishView {
  ack: AckView;
  mdln: string;
  softrev: string;
  control: ControlView | null;
}

export interface OnlineView {
  ack: AckView;
  control: ControlView | null;
}

export interface SvRow {
  id: number;
  name: string;
  units: string;
}

export type SvValue = ValueView & { id: number };

export type EcRow = ValueView & {
  id: number;
  name: string;
  units: string;
  min: number | null;
  max: number | null;
  default: string;
};

export interface EcWriteView {
  ack: AckView;
  value: ValueView | null;
}

export interface WizardStep {
  step: string;
  ack: AckView;
}

export interface AlarmView {
  alid: number;
  set: boolean;
  category: number;
  text: string;
}

export interface ParamIn {
  name: string;
  kind: string;
  value: string;
}

export interface CommandView {
  hcack: AckView;
  params: { name: string; cpack: AckView }[];
}

export interface RawReply {
  name: string | null;
  sml: string | null;
}

/** One message of the HSMS message log (`gem-msg`). */
export interface MsgEntry {
  source: "host" | "sim";
  timeMs: number;
  dir: "tx" | "rx";
  /** `S1F13` for data messages, `Select.req` and so on for control messages. */
  kind: string;
  system: number;
  /** One-line SML (data) or the control message's status. */
  text: string;
  /** Indented SML (data messages). */
  pretty: string;
  data: boolean;
}

/** Something the equipment sent by itself (`gem-event`). */
export type GemEvent =
  | { kind: "event"; dataid: number; ceid: number; reports: { rptid: number; values: ValueView[] }[] }
  | { kind: "alarm"; alid: number; set: boolean; category: number; text: string }
  | { kind: "terminal"; tid: number; text: string }
  | { kind: "s9"; text: string }
  | { kind: "unhandled"; text: string }
  | { kind: "closed"; reason: string | null };

export interface SimSv {
  id: number;
  name: string;
  units: string;
  text: string;
  editable: boolean;
}

export interface SimEc {
  id: number;
  name: string;
  units: string;
  min: number;
  max: number;
  value: number;
  locked: boolean;
}

export interface SimView {
  port: number;
  selected: boolean;
  sessions: number;
  requests: number;
  control: ControlView;
  onlineRemote: boolean;
  comm: string;
  commEnabled: boolean;
  process: {
    state: "Idle" | "Executing" | "Paused";
    ppid: string;
    lotId: string;
    wafers: number;
    elapsedS: number;
    durationS: number;
    pressure: number;
    temperature: number;
  };
  svs: SimSv[];
  constants: SimEc[];
  alarms: { alid: number; text: string; category: number; enabled: boolean; set: boolean }[];
  events: { ceid: number; name: string; enabled: boolean }[];
  ppids: string[];
  terminal: string[];
}

/** A report the host defined in this session, kept to name the values of S6F11. */
export interface LocalReport {
  rptid: number;
  vids: number[];
  ceids: number[];
}

/** HSMS connection settings as the form holds them (timers in seconds). */
export interface HostForm {
  mode: "active" | "passive";
  host: string;
  port: number;
  deviceId: number;
  t3: number;
  t5: number;
  t6: number;
  t7: number;
  t8: number;
  controlSvid: number;
}

/** What `gem_connect` takes (timers in milliseconds). */
export interface HostConnect {
  mode: "active" | "passive";
  host: string;
  port: number;
  deviceId: number;
  t3Ms: number;
  t5Ms: number;
  t6Ms: number;
  t7Ms: number;
  t8Ms: number;
  controlSvid: number;
}
