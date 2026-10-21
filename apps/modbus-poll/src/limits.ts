import { FREE_CONNECTIONS, FREE_VALUES } from "./constants";
import { buildReadPlan, type ReadPlan } from "./helpers";
import type { ReadBlock } from "./types";

/** Values a block list polls: one per value, however many registers it spans. */
export const valueCount = (blocks: ReadBlock[]) => blocks.reduce((n, b) => n + b.quantity, 0);

/** How many values may be polled at once. */
export const polledLimit = (pro: boolean) => (pro ? Infinity : FREE_VALUES);

/** The first `max` values of a plan. `firstTag` keeps pointing at the full list, so rows past the cap read nothing. */
export function limitReadPlan(plan: ReadPlan, max: number): ReadPlan {
  return plan.tags.length <= max ? plan : { tags: plan.tags.slice(0, max), firstTag: plan.firstTag };
}

/** The plan that is actually polled for `blocks`: all of it with Pro, the first FREE_VALUES values without. */
export const effectivePlan = (blocks: ReadBlock[], pro: boolean): ReadPlan =>
  limitReadPlan(buildReadPlan(blocks), polledLimit(pro));

/** True when `blocks` hold more values than the free version polls. */
export const exceedsFreeValues = (blocks: ReadBlock[], pro: boolean) => !pro && valueCount(blocks) > FREE_VALUES;

/** Shown while a block list is over the free cap; null when everything is polled. */
export function valueCapNotice(blocks: ReadBlock[], pro: boolean): string | null {
  if (!exceedsFreeValues(blocks, pro)) return null;
  return `Polling the first ${FREE_VALUES} of ${valueCount(blocks)} values. Pro polls all of them.`;
}

/** Message for the comm log when a saved profile is loaded into the free version with too many values. */
export function profileLoadNotice(name: string, blocks: ReadBlock[], pro: boolean): string | null {
  if (!exceedsFreeValues(blocks, pro)) return null;
  return `Profile '${name}' has ${valueCount(blocks)} values. The free version polls the first ${FREE_VALUES}; the rest show as Pro. Unlock Pro to poll them all.`;
}

/** Another connection may be opened: Pro always, free only while below the free count. */
export const mayOpenConnection = (openCount: number, pro: boolean) => pro || openCount < FREE_CONNECTIONS;
