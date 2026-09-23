// WebSerial ESP32 provisioning UI (PLC-26). Flashes the runtime image from
// the release manifest over USB and surfaces the pairing token issued by
// POST /api/devices. NVS provisioning is not possible with esptool-js (raw
// flash writes only), so that step shows honest manual instructions instead.
import { useEffect, useState } from "react";
import { useCloudStore } from "../../store/cloud";
import {
  APP_IMAGE_OFFSET,
  PROVISION_BASE_KEY,
  buildFlashPlan,
  cliFallbackCommand,
  downloadVerifiedImage,
  fetchManifest,
  isWebSerialAvailable,
  provisionBase,
  registerDevice,
  type RegisteredDevice,
  type RuntimeManifest,
} from "../../api/provision";

type FlashPhase = "idle" | "connecting" | "downloading" | "flashing" | "done";

const LOG_LIMIT = 200;

type Log = (line: string) => void;
type Progress = (loaded: number, total: number) => void;

// Dynamic import is required: esptool-js (and its pako payload) must land in
// a lazy chunk that non-Chromium bundles never download, so the specifier is
// deliberately not hoisted to a static import.
async function flashRuntimeImage(manifest: RuntimeManifest, log: Log, setPhase: (p: FlashPhase) => void, onProgress: Progress): Promise<void> {
  const { ESPLoader, Transport } = await import("esptool-js");
  const port = await navigator.serial.requestPort();
  const transport = new Transport(port, true);
  try {
    const loader = new ESPLoader({
      transport,
      baudrate: 115200,
      romBaudrate: 115200,
      terminal: {
        clean: () => {},
        write: (data: string) => log(data.replace(/\n+$/, "")),
        writeLine: (line: string) => log(line),
      },
    });
    const chipName = await loader.main();
    log(`Connected: ${chipName}`);

    const entry = buildFlashPlan(manifest)[0];
    setPhase("downloading");
    const image = await downloadVerifiedImage(entry, onProgress);
    log(`Image verified: ${image.length} bytes, sha256 ${entry.sha256.slice(0, 12)}…`);

    setPhase("flashing");
    await loader.writeFlash({
      fileArray: [{ data: image, address: entry.address }],
      flashSize: "keep",
      flashMode: "keep",
      flashFreq: "keep",
      eraseAll: false,
      compress: true,
      reportProgress: (_fileIndex, written, total) => onProgress(written, total),
    });
    await loader.after();
  } finally {
    await transport.disconnect().catch(() => {});
  }
}

export default function ProvisionPanel() {
  const token = useCloudStore((s) => s.token);

  const [serialOk] = useState(isWebSerialAvailable);
  const [manifest, setManifest] = useState<RuntimeManifest | null>(null);
  const [manifestError, setManifestError] = useState<string | null>(null);

  const [deviceName, setDeviceName] = useState("");
  const [registering, setRegistering] = useState(false);
  const [device, setDevice] = useState<RegisteredDevice | null>(null);

  const [phase, setPhase] = useState<FlashPhase>("idle");
  const [progress, setProgress] = useState<{ loaded: number; total: number } | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchManifest()
      .then((m) => {
        if (!cancelled) setManifest(m);
      })
      .catch((e) => {
        if (!cancelled) setManifestError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const busy = phase !== "idle" && phase !== "done";

  const appendLog: Log = (line) => setLog((prev) => [...prev.slice(-(LOG_LIMIT - 1)), line]);

  async function register() {
    if (!token || deviceName.trim().length === 0) return;
    setRegistering(true);
    setError(null);
    try {
      setDevice(await registerDevice(token, deviceName.trim()));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRegistering(false);
    }
  }

  async function connectAndFlash() {
    if (!manifest || busy) return;
    setPhase("connecting");
    setError(null);
    setProgress(null);
    setLog([]);
    try {
      await flashRuntimeImage(manifest, appendLog, setPhase, (loaded, total) => setProgress({ loaded, total }));
      setPhase("done");
    } catch (e) {
      setPhase("idle");
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const pct =
    progress && progress.total > 0 ? Math.min(100, Math.round((progress.loaded / progress.total) * 100)) : 0;

  return (
    <div style={{ padding: "0 12px 10px", display: "flex", flexDirection: "column", gap: 6, fontSize: 11 }}>
      <div style={{ fontWeight: 600, marginTop: 4 }}>Device provisioning</div>

      {/* Step 1: browser + manifest */}
      <div style={{ color: "var(--text-muted)" }}>
        1. Browser: {serialOk ? "Web Serial available" : "Web Serial NOT available"} · Manifest:{" "}
        {manifest ? `runtime v${manifest.version} (${manifest.chip})` : manifestError ? "failed" : "loading…"}
      </div>
      {manifestError && (
        <div role="alert" style={{ color: "var(--danger)" }}>
          Manifest failed: {manifestError}
          <div style={{ color: "var(--text-muted)", marginTop: 2 }}>
            Base: {provisionBase()} — override with localStorage key {PROVISION_BASE_KEY}
          </div>
        </div>
      )}
      {!serialOk && (
        <div style={{ color: "var(--amber)" }}>
          <div>
            This browser has no Web Serial. Use Chrome/Edge/Chromium to flash over USB, or flash manually:
          </div>
          <div style={{ fontFamily: "monospace", margin: "2px 0" }}>{cliFallbackCommand()}</div>
          {manifest && (
            <a href={manifest.url} download style={{ color: "var(--accent)" }}>
              Download runtime image v{manifest.version} ({Math.ceil(manifest.size / 1024)} KB)
            </a>
          )}
        </div>
      )}

      {/* Step 2: register device, get pairing token */}
      <div style={{ fontWeight: 600, marginTop: 4 }}>2. Register device</div>
      {!token ? (
        <div style={{ color: "var(--text-muted)" }}>Sign in to provision devices.</div>
      ) : device ? (
        <>
          <div>
            Registered <span style={{ fontWeight: 600 }}>{device.name}</span> ({device.id})
          </div>
          <div style={{ fontFamily: "monospace", wordBreak: "break-all" }}>{device.pairingToken}</div>
          <div style={{ color: "var(--amber)" }}>Pairing token is shown only once — store it now.</div>
        </>
      ) : (
        <div style={{ display: "flex", gap: 4 }}>
          <input
            type="text"
            placeholder="Device name"
            aria-label="Device name"
            value={deviceName}
            onChange={(e) => setDeviceName(e.target.value)}
            style={{ flex: 1, fontSize: 11, padding: "2px 4px" }}
          />
          <button
            className="toolbar-btn"
            style={{ fontSize: 10, padding: "0 8px" }}
            disabled={registering || deviceName.trim().length === 0}
            onClick={() => void register()}
          >
            {registering ? "…" : "Register"}
          </button>
        </div>
      )}

      {/* Step 3: connect + flash (Chromium only) */}
      {serialOk && (
        <>
          <div style={{ fontWeight: 600, marginTop: 4 }}>3. Flash runtime</div>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <button
              className="toolbar-btn"
              style={{ fontSize: 10, padding: "0 8px" }}
              disabled={!manifest || !device || busy}
              onClick={() => void connectAndFlash()}
            >
              {phase === "idle" ? "Connect & flash" : phase === "done" ? "Reflash" : `${phase}…`}
            </button>
            {progress && busy && (
              <span style={{ color: "var(--text-muted)" }}>
                {pct}% ({progress.loaded}/{progress.total})
              </span>
            )}
          </div>
          <div style={{ color: "var(--text-muted)" }}>
            App image at offset 0x{APP_IMAGE_OFFSET.toString(16)} · erase-all off — existing NVS keys survive.
          </div>
        </>
      )}

      {/* Step 4: provisioning — esptool-js has no NVS writer, so this is manual */}
      <div style={{ fontWeight: 600, marginTop: 4 }}>4. Provision (manual)</div>
      <div style={{ color: "var(--text-muted)" }}>
        esptool-js writes raw flash only — it cannot build or write NVS partitions in-browser. After
        flashing, provision the board out-of-band:
        <ol style={{ margin: "4px 0 0", paddingLeft: 16 }}>
          <li>
            Open the board's USB serial console (e.g. <span style={{ fontFamily: "monospace" }}>screen</span>,{" "}
            <span style={{ fontFamily: "monospace" }}>minicom</span>, or a Web Serial terminal) and enter the
            Wi-Fi credentials plus the pairing token from step 2.
          </li>
          <li>
            Alternatively, generate an NVS image offline with{" "}
            <span style={{ fontFamily: "monospace" }}>nvs_partition_gen.py</span> and flash it with{" "}
            <span style={{ fontFamily: "monospace" }}>esptool.py --chip esp32s3 write_flash &lt;nvs-offset&gt; nvs.bin</span>.
          </li>
        </ol>
      </div>
      {device && (
        <div style={{ color: "var(--text-muted)" }}>
          Once the device connects, it appears under Devices in the cloud dashboard (
          <span style={{ fontFamily: "monospace" }}>GET /api/devices</span>).
        </div>
      )}

      {phase === "done" && <div style={{ color: "var(--accent)" }}>Flash complete — device reset.</div>}
      {error && (
        <div role="alert" style={{ color: "var(--danger)" }}>
          {error}
        </div>
      )}
      {log.length > 0 && (
        <details>
          <summary style={{ cursor: "pointer", color: "var(--text-muted)" }}>esptool log</summary>
          <pre
            style={{
              maxHeight: 120,
              overflow: "auto",
              margin: "2px 0 0",
              fontSize: 10,
              fontFamily: "monospace",
              whiteSpace: "pre-wrap",
            }}
          >
            {log.join("\n")}
          </pre>
        </details>
      )}
    </div>
  );
}
