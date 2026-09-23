// The updater/process plugins are Tauri-webview-only: static imports would pull
// them into the web bundle, so they are loaded inside these functions.
export async function checkForUpdates(): Promise<{
  available: boolean;
  version?: string;
  notes?: string;
}> {
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
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
    const { check } = await import("@tauri-apps/plugin-updater");
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

    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
    return true;
  } catch {
    return false;
  }
}
