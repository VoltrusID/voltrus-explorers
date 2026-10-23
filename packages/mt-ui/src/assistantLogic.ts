// Pure parts of the Connect AI assistant UI: wire types (mirror the console contract, voltrus-console
// docs/connect-ai.md), input parsing and the wording of failures and quota. No React and no Tauri here, so
// the apps' unit tests can import it.

export type AssistantProtocol = "modbus" | "bacnet";
export type AssistantTask = "lookup" | "manual" | "diagnose" | "decode" | "ask";

export type Area = "holding" | "input" | "coil" | "discrete";
export type BlockType = "uint16" | "int16" | "int32" | "float32" | "float64";
export type WordOrder = "big" | "little" | "swapped";

export interface BlockSource {
  /** Absent when the source is text the user supplied. */
  url?: string;
  title: string;
  origin: "web" | "user";
  /** The manufacturer's own site. */
  vendor: boolean;
  page?: string;
  quote: string;
}

/** A proposed read block. The assistant has no field for a write. */
export interface AssistantBlock {
  name: string;
  area: Area;
  /** Zero-based protocol address. */
  address: number;
  quantity: number;
  type: BlockType;
  word_order: WordOrder;
  scale?: number;
  unit?: string;
  /** The number as the manual prints it. */
  documented_address?: string;
  note?: string;
  confidence: "high" | "medium" | "low";
  source?: BlockSource;
}

export interface KindQuota {
  used: number;
  limit: number;
  remaining: number;
}

export interface AssistantQuota {
  period: string;
  /** ms epoch of the next reset. */
  resets_at: number;
  text: KindQuota;
  image: KindQuota;
}

export interface AssistantStatus {
  enabled: boolean;
  entitled?: boolean;
  quota?: AssistantQuota;
}

/** What `connect_ai_run` rejects with. */
export interface AssistantFailure {
  /** HTTP status; 0 when the server could not be reached. */
  status: number;
  code: string;
  message: string;
  quota?: AssistantQuota | null;
}

export interface RegisterMapResult {
  status: "found" | "no_source";
  blocks: AssistantBlock[];
  sources: { url?: string; title: string; vendor: boolean }[];
  notes?: string;
  needs_manual?: true;
}

export interface ManualResult {
  blocks: AssistantBlock[];
  notes?: string;
}

export interface Diagnosis {
  summary: string;
  causes: { title: string; detail: string; likelihood: "high" | "medium" | "low" }[];
  checks: string[];
  suggestions: { setting: string; from?: string | number | boolean; to: string | number | boolean; reason: string }[];
}

export interface DecodeCandidate {
  register_index: number;
  type: BlockType;
  word_order: WordOrder;
  scale: number;
  raw: number;
  value: number;
  reason: string;
  confidence: "high" | "medium" | "low";
}

export interface AnswerResult {
  answer: string;
  follow_ups: string[];
}

/** What an app tells the assistant about the current session (the server masks hosts, IPs and secrets). */
export interface AssistantContext {
  config?: unknown;
  /** Last log lines, oldest first. */
  log?: string[];
  /** The block or object the user is looking at. */
  block?: unknown;
  /** The last error the app showed, if any. */
  error?: { kind: "exception" | "timeout" | "other"; code?: number; message?: string };
}

/** Bump when the consent text changes: everyone is asked again. */
export const CONSENT_VERSION = 1;
export const consentKey = (slug: string) => `voltrus-assistant-consent:${slug}`;

export const WORD_ORDER_LABEL: Record<WordOrder, string> = {
  big: "AB CD",
  little: "CD AB",
  swapped: "BA DC",
};

/** "16840 0", "0x41C8, 0x0000" -> registers; a string is the reason the text can not be used. */
export function parseRegisters(text: string): number[] | string {
  const parts = text.split(/[\s,;]+/).filter(Boolean);
  if (parts.length === 0) return "Enter the register values, separated by spaces or commas.";
  if (parts.length > 16) return "At most 16 registers.";
  const out: number[] = [];
  for (const p of parts) {
    const n = /^0x[0-9a-f]{1,4}$/i.test(p) ? parseInt(p, 16) : /^\d{1,5}$/.test(p) ? Number(p) : NaN;
    if (!Number.isInteger(n) || n < 0 || n > 65535) return `"${p}" is not a register value (0 to 65535, or hex like 0x41C8).`;
    out.push(n);
  }
  return out;
}

/** A Modbus exception code as people write it: "2", "02", "0x0B", "0B" (letters mean hex). */
export function parseErrorCode(text: string): number | undefined {
  const t = text.trim();
  const n = /^0x[0-9a-f]{1,2}$/i.test(t)
    ? parseInt(t, 16)
    : /^[0-9a-f]{1,2}$/i.test(t) && /[a-f]/i.test(t)
      ? parseInt(t, 16)
      : /^\d{1,3}$/.test(t)
        ? Number(t)
        : NaN;
  return Number.isInteger(n) && n >= 0 && n <= 255 ? n : undefined;
}

export function resetDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** "Text 296 of 300 left, images 29 of 30 left. Resets Nov 1." */
export function quotaLine(q: AssistantQuota | null | undefined): string | null {
  if (!q) return null;
  return `Text ${q.text.remaining} of ${q.text.limit} left, photos ${q.image.remaining} of ${q.image.limit} left. Resets ${resetDate(q.resets_at)}.`;
}

export type FailureKind = "signin" | "upgrade" | "quota" | "info";

/** Wording and the follow-up the UI offers for a refused or failed request. */
export function failureInfo(f: AssistantFailure): { kind: FailureKind; text: string } {
  switch (f.code) {
    case "not_signed_in":
    case "invalid_token":
      return { kind: "signin", text: "Sign in to Voltrus Connect to use the Assistant." };
    case "entitlement_required":
      return { kind: "upgrade", text: "The Assistant is included with Voltrus Connect." };
    case "quota_exceeded": {
      const when = f.quota ? ` It resets ${resetDate(f.quota.resets_at)}.` : "";
      return { kind: "quota", text: `You have used this month's Assistant requests.${when}` };
    }
    case "ai_unavailable":
      return f.status === 403
        ? { kind: "info", text: "The Assistant is not available for this account yet." }
        : { kind: "info", text: "The Assistant is not reachable right now. This request was not counted. Try again in a moment." };
    case "search_unavailable":
      return { kind: "info", text: "Searching for the manual is not available right now. Paste the register table instead." };
    case "invalid_model_output":
      return { kind: "info", text: "The Assistant did not give a usable answer. This request was not counted. Try again, or rephrase." };
    case "network":
      return { kind: "info", text: "Could not reach Voltrus. Check your connection and try again." };
    case "too_large":
      return { kind: "info", text: "That is too large to send. Use less text or a smaller photo." };
    case "invalid_request":
      return { kind: "info", text: f.message };
    default:
      return { kind: "info", text: f.message || "Something went wrong. This request was not counted." };
  }
}

/** One line per block for the review list: "Voltage L1-N  holding 3027 x2 float32 (manual says 3028)". */
export function blockSummary(b: AssistantBlock): string {
  const scale = b.scale !== undefined ? ` x${b.scale}` : "";
  const unit = b.unit ? ` ${b.unit}` : "";
  const printed =
    b.documented_address && b.documented_address !== String(b.address) ? ` (manual says ${b.documented_address})` : "";
  return `${b.area} ${b.address}, ${b.quantity} ${b.quantity === 1 ? "register" : "registers"}, ${b.type}${scale}${unit}${printed}`;
}

/** The word order to suggest when the chosen blocks agree on one that is not the Modbus default; otherwise null. */
export function commonWordOrder(blocks: AssistantBlock[]): WordOrder | null {
  const multi = blocks.filter((b) => b.quantity > 1 && b.area !== "coil" && b.area !== "discrete");
  const first = multi[0]?.word_order;
  return first && first !== "big" && multi.every((b) => b.word_order === first) ? first : null;
}

export function isFailure(e: unknown): e is AssistantFailure {
  return typeof e === "object" && e !== null && "code" in e && typeof e.code === "string" && "status" in e;
}
