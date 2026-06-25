import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export async function checkForUpdates(): Promise<{
  available: boolean;
  version?: string;
  notes?: string;
}> {
  try {
    const update = await check();
    if (update) {
      return {
        available: true,
        version: update.version,
        notes: update.body ?? undefined,
      };
    }
    return { available: false };
  } catch {
    return { available: false };
  }
}

export async function installUpdate(
  onProgress?: (percent: number) => void,
): Promise<boolean> {
  try {
    const update = await check();
    if (!update) return false;

    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          break;
        case "Progress":
          if (onProgress) {
            onProgress(Math.min(99, Math.round(event.data.chunkLength / 100)));
          }
          break;
        case "Finished":
          if (onProgress) onProgress(100);
          break;
      }
    });

    await relaunch();
    return true;
  } catch {
    return false;
  }
}
