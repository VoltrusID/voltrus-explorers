import type { ConnProfile, ReadBlock } from "./types";
import { AREAS, TYPES } from "./constants";

/** Registers one value of `type` occupies. */
export const typeStep = (t: string) =>
  t === "float64" ? 4 : t === "float32" || t === "int32" ? 2 : 1;

/** Coil and discrete-input blocks address single bits; they have no register type. */
export const isBitArea = (area: string) => area === "coil" || area === "discrete";

/** Areas a master can write to (FC05/06/15/16). Input registers and discrete inputs are read-only. */
export const isWritableArea = (area: string) => area === "holding" || area === "coil";

/** The type a block is actually read with. */
export const blockType = (b: Pick<ReadBlock, "type" | "area">) => (isBitArea(b.area) ? "uint16" : b.type);

/** Address distance between consecutive values of a block. */
export const blockStep = (b: Pick<ReadBlock, "type" | "area">) => typeStep(blockType(b));

/** Name of the `i`-th value of a block, e.g. `Voltage_3001`. */
export const valueName = (b: ReadBlock, i: number) =>
  `${b.name}_${(b.address + i * blockStep(b)).toString().padStart(3, "0")}`;

/** One entry of the `read_tags` request: one value, however many registers it spans. */
export interface ReadTag {
  name: string;
  address: number;
  quantity: number;
  area: string;
  register_area: string;
  type: string;
  scale: number;
}

export interface ReadPlan {
  /** One tag per displayed value, in block order. `read_tags` returns one value per tag. */
  tags: ReadTag[];
  /** `firstTag[blockIdx]` is the index in `tags` (and in the returned values) of that block's first value. */
  firstTag: number[];
}

export function buildReadPlan(blocks: ReadBlock[]): ReadPlan {
  const tags: ReadTag[] = [];
  const firstTag: number[] = [];
  blocks.forEach((b) => {
    firstTag.push(tags.length);
    const step = blockStep(b);
    for (let i = 0; i < b.quantity; i++) {
      tags.push({
        name: valueName(b, i),
        address: b.address + i * step,
        quantity: step,
        area: b.area,
        register_area: b.area,
        type: blockType(b),
        scale: 1.0,
      });
    }
  });
  return { tags, firstTag };
}

/** Identity of a plan: values only line up with rows while the tag lists are the same. */
export const readPlanKey = (plan: ReadPlan) =>
  plan.tags.map((t) => `${t.register_area}:${t.address}:${t.type}`).join("|");

/** Why a block can not be polled, or null if it can. */
export function validateBlock(b: ReadBlock): string | null {
  if (!b.name.trim()) return "Block name is empty";
  if (!TYPES.includes(b.type)) return `Unsupported type '${b.type}' (use ${TYPES.join(", ")})`;
  if (!AREAS.includes(b.area)) return `Unsupported area '${b.area}' (use ${AREAS.join(", ")})`;
  if (!Number.isInteger(b.address) || b.address < 0 || b.address > 65535) return `Address ${b.address} is not an integer in 0-65535`;
  if (!Number.isInteger(b.quantity) || b.quantity < 1) return `Quantity ${b.quantity} is not a positive integer`;
  const last = b.address + b.quantity * blockStep(b) - 1;
  if (last > 65535) return `Block runs past address 65535 (ends at ${last})`;
  return null;
}

export const CONN_PROFILES_KEY = "modbus-poll-conn-profiles";

export function loadConnProfiles(): ConnProfile[] {
  try {
    const raw = localStorage.getItem(CONN_PROFILES_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

export function saveConnProfiles(profiles: ConnProfile[]) {
  localStorage.setItem(CONN_PROFILES_KEY, JSON.stringify(profiles));
}
