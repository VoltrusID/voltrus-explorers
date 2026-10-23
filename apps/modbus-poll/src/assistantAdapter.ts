// Modbus Poll's side of the Assistant: what the app tells it about the session, and how its proposed blocks
// become ReadBlocks. Pure functions; the component (components/AssistantHost.tsx) wires them to the store.
import type { AssistantBlock, AssistantContext, WordOrder } from "@mactools/ui";
import { isBitArea, typeStep, validateBlock } from "./helpers";
import type { ReadBlock } from "./types";

/**
 * A read failure message as the app shows it ("Read failed: Illegal Data Address (0x02) reading holding 0..9")
 * as the Assistant's error: an exception with its code, a timeout, or something else.
 */
export function assistantError(message: string | null | undefined): AssistantContext["error"] {
  if (!message) return undefined;
  const code = /\(0x([0-9a-f]{2})\)/i.exec(message);
  if (code) return { kind: "exception", code: parseInt(code[1]!, 16), message: message.slice(0, 300) };
  if (/time(d)? ?out/i.test(message)) return { kind: "timeout", message: message.slice(0, 300) };
  return { kind: "other", message: message.slice(0, 300) };
}

export interface SessionFacts {
  transport: "tcp" | "rtu";
  tcp: Record<string, string | number>;
  rtu: { baudRate: number; dataBits: number; stopBits: number; parity: string; unitId: number; timeoutMs: number; pollIntervalMs: number };
  endianness: string;
  oneBasedDisplay: boolean;
  blocks: ReadBlock[];
}

/** Connection facts for the Assistant. The server drops hosts and credential-like keys as well; leaving them out here keeps them on the device. */
export function sessionConfig(f: SessionFacts): Record<string, unknown> {
  const { host: _host, ...tcp } = f.tcp;
  return {
    transport: f.transport,
    ...(f.transport === "tcp" ? { tcp } : { rtu: f.rtu }),
    wordOrder: f.endianness,
    addressesShownFrom: f.oneBasedDisplay ? 1 : 0,
    blocks: f.blocks.slice(0, 20).map((b) => ({ name: b.name, area: b.area, address: b.address, quantity: b.quantity, type: b.type })),
  };
}

export interface BlocksOutcome {
  blocks: ReadBlock[];
  /** Why a proposal could not become a block. */
  skipped: string[];
}

/**
 * Proposed blocks as ReadBlocks. The Assistant counts registers; a ReadBlock counts values, so a float32 over
 * 6 registers is 3 values. Names are made unique against what is already there.
 */
export function toReadBlocks(proposals: AssistantBlock[], existing: ReadBlock[]): BlocksOutcome {
  const taken = new Set(existing.map((b) => b.name.toLowerCase()));
  const blocks: ReadBlock[] = [];
  const skipped: string[] = [];
  for (const p of proposals) {
    const step = isBitArea(p.area) ? 1 : typeStep(p.type);
    if (p.quantity % step !== 0) {
      skipped.push(`${p.name}: ${p.quantity} registers do not fit ${p.type}`);
      continue;
    }
    let name = p.name.trim();
    for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${p.name.trim()} ${n}`;
    const block: ReadBlock = {
      name,
      address: p.address,
      quantity: p.quantity / step,
      type: isBitArea(p.area) ? "uint16" : p.type,
      area: p.area,
      ...(p.scale !== undefined && { scale: p.scale }),
    };
    const problem = validateBlock(block);
    if (problem) {
      skipped.push(`${p.name}: ${problem}`);
      continue;
    }
    taken.add(name.toLowerCase());
    blocks.push(block);
  }
  return { blocks, skipped };
}

/** The sentence shown after adding: what was added, what was left out and what the app can not do for the user. */
export function addedMessage(added: ReadBlock[], skipped: string[], order: WordOrder | null, currentOrder: string): string {
  const parts = [`Added ${added.length} ${added.length === 1 ? "block" : "blocks"}.`];
  if (skipped.length > 0) parts.push(`Left out: ${skipped.join("; ")}.`);
  if (added.some((b) => b.scale !== undefined))
    parts.push("Modbus Poll shows raw values: apply the manual's scale factors yourself.");
  if (order && order !== currentOrder) parts.push(`Set the word order to ${order === "little" ? "CD AB" : "BA DC"} in the connection settings.`);
  return parts.join(" ");
}
