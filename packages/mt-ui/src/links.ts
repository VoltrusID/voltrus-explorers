import { invoke } from "@tauri-apps/api/core";
import { getIdentifier } from "@tauri-apps/api/app";

export type UtmPlatform = "macos" | "ios" | "windows" | "linux";

export function detectPlatform(): UtmPlatform {
  const nav = typeof navigator === "undefined" ? undefined : navigator;
  const ua = nav?.userAgent ?? "";
  if (/iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && (nav?.maxTouchPoints ?? 0) > 1)) return "ios";
  if (/Windows/.test(ua)) return "windows";
  if (/Macintosh|Mac OS X/.test(ua)) return "macos";
  return "linux";
}

function isVoltrusHost(host: string): boolean {
  return host === "voltrus.id" || host.endsWith(".voltrus.id");
}

/**
 * Pure form of [`withUtm`]: adds `utm_source`, `utm_medium=app`, `utm_campaign` and `utm_content` to voltrus.id
 * and *.voltrus.id URLs. Other hosts, unparsable URLs and parameters already present are left alone.
 */
export function tagUrl(url: string, campaign: string, source: string, platform: UtmPlatform): string {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return url;
  }
  if ((u.protocol !== "https:" && u.protocol !== "http:") || !isVoltrusHost(u.hostname)) return url;
  const add: [string, string][] = [
    ["utm_source", source],
    ["utm_medium", "app"],
    ["utm_campaign", campaign],
    ["utm_content", platform],
  ];
  for (const [k, v] of add) if (!u.searchParams.has(k)) u.searchParams.set(k, v);
  return u.toString();
}

let sourcePromise: Promise<string> | null = null;

/** The app slug, e.g. `modbus-poll`: the last label of the bundle identifier (`com.joyodigitama.modbus-poll`). */
function appSlug(): Promise<string> {
  sourcePromise ??= getIdentifier().then((id) => id.split(".").pop() || id);
  return sourcePromise;
}

/** Tags a voltrus.id link so RevKlik can attribute the visit; `campaign` names the link's purpose. */
export async function withUtm(url: string, campaign: string): Promise<string> {
  try {
    return tagUrl(url, campaign, await appSlug(), detectPlatform());
  } catch {
    return url;
  }
}

/**
 * Opens `url` in the system browser, tagged with UTM parameters when it is a voltrus.id link. Apps with Voltrus Connect
 * register `connect_open_url` (opener plugin, also on iOS); every app registers its own `open_url`, used otherwise.
 */
export async function openExternal(url: string, campaign: string): Promise<void> {
  const tagged = await withUtm(url, campaign);
  try {
    await invoke("connect_open_url", { url: tagged });
  } catch (e) {
    if (!String(e).includes("not found")) throw e;
    await invoke("open_url", { url: tagged });
  }
}
