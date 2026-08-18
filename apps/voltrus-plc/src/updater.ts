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

    let downloaded = 0;
    let total = 0;
    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          downloaded = 0;
          total = event.data.contentLength ?? 0;
          if (onProgress) onProgress(0);
          break;
        case "Progress":
          if (onProgress && total > 0) {
            downloaded += event.data.chunkLength;
            onProgress(Math.min(99, Math.round((downloaded / total) * 100)));
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
