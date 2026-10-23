import type { ReactNode } from "react";

/** One choice of a `SheetSelect`. */
export interface SheetOption<T extends string | number = string> {
  value: T;
  /** Shown on the row and, when the control is closed, in the trigger. Plain text (it is also what type-ahead matches). */
  label: string;
  /** Description: a second line under the label (a template's summary, an address range). Searchable. */
  hint?: string;
  /** Header the row sits under. Neighbouring rows with the same group are drawn together; the order of the list is kept. */
  group?: string;
  disabled?: boolean;
  /** A glyph before the label, in the list and in the trigger. */
  icon?: ReactNode;
  /** Phone sheet only: a control after the label, e.g. a close button. Stop its click from reaching the row, or it also picks the row. */
  trailing?: ReactNode;
}

/** A list with more choices than this is a searchable combobox (a search field on the phone sheet) instead of a plain select. */
export const SEARCH_THRESHOLD = 5;

export function isSearchable(count: number, searchable: boolean | undefined): boolean {
  return searchable ?? count > SEARCH_THRESHOLD;
}

/** Options whose label, hint or group contains `query` (case-insensitive, whitespace-trimmed); all of them for an empty query. */
export function filterSheetOptions<T extends string | number>(options: ReadonlyArray<SheetOption<T>>, query: string): SheetOption<T>[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...options];
  return options.filter((o) => `${o.label} ${o.hint ?? ""} ${o.group ?? ""}`.toLowerCase().includes(q));
}

export interface SheetGroup<T extends string | number> {
  group: string | undefined;
  items: SheetOption<T>[];
}

/** Splits options into runs of neighbouring rows with the same `group` (undefined = no header), so the order of the list is kept. */
export function groupSheetOptions<T extends string | number>(options: ReadonlyArray<SheetOption<T>>): SheetGroup<T>[] {
  const out: SheetGroup<T>[] = [];
  for (const o of options) {
    const last = out[out.length - 1];
    if (last && last.group === o.group) last.items.push(o);
    else out.push({ group: o.group, items: [o] });
  }
  return out;
}
