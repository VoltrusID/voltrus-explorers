import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface UpdateInfo {
  available: boolean;
  version?: string;
  notes?: string;
}

export async function checkForUpdates(): Promise<UpdateInfo> {
  const update = await check();
  if (update) {
    return { available: true, version: update.version, notes: update.body ?? undefined };
  }
  return { available: false };
}

export async function installUpdate(onProgress?: (percent: number) => void): Promise<boolean> {
  const update = await check();
  if (!update) return false;

  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        break;
      case "Progress":
        if (onProgress) onProgress(Math.min(99, Math.round((event.data.chunkLength / 100) % 100)));
        break;
      case "Finished":
        if (onProgress) onProgress(100);
        break;
    }
  });

  await relaunch();
  return true;
}