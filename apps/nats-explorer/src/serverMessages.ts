export interface ServerRecentLike {
  subject: string;
  payload: string;
  timestamp: number;
  headers: [string, string][];
}

export const SERVER_MESSAGE_CAP = 200;

/**
 * Build the server message list from the server's own recent buffer.
 * The buffer already holds every routed message, so the list is rebuilt from
 * it on each poll instead of being merged into the previous list (a merge
 * re-added the same messages every second). `since` hides messages the user
 * cleared; it is compared against the server timestamp in milliseconds.
 */
export function serverMessagesFromRecent<T extends ServerRecentLike>(
  recent: T[],
  since: number,
  cap: number = SERVER_MESSAGE_CAP,
): (T & { _ts: number })[] {
  return recent
    .filter((r) => r.timestamp > since)
    .slice(-cap)
    .map((r) => ({ ...r, _ts: r.timestamp }));
}
